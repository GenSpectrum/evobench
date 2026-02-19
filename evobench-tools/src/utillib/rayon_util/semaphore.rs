use std::{
    num::NonZeroU32,
    sync::atomic::{AtomicU32, Ordering},
    thread,
};

use crate::debug;

pub struct Semaphore(AtomicU32);

pub struct SemaphoreHolder<'t>(&'t Semaphore);

impl<'t> Drop for SemaphoreHolder<'t> {
    fn drop(&mut self) {
        self.0.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Semaphore {
    pub fn new(parallelism: NonZeroU32) -> Self {
        Self(parallelism.get().into())
    }

    /// Meant to be run from a thread in a rayon thread pool, so that
    /// it can efficiently yield to other rayon work. If you give
    /// `panic_if_not_rayon == true` then it will panic if this is not
    /// the case.
    pub fn acquire_rayon(&self, panic_if_not_rayon: bool) -> SemaphoreHolder<'_> {
        let mut os_yields: usize = 0;
        loop {
            // Tempting: instead use signed atomic, decrement with
            // atomic dec, then check if the old value is >= 0, and if
            // not increment again. But that leads to potentially
            // multiple parties trying to acquire to all need to
            // increment again (plus a drop from a holder) until one
            // can succeed again, which seems problematic; and it will
            // always have two writes at the limit, which might even
            // make it slower without the scheduling problem. Thus,
            // leave it at fetch_update.
            if self
                .0
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |old| old.checked_sub(1))
                .is_ok()
            {
                if os_yields > 0 {
                    debug!("acquire_rayon: yielded to the OS {os_yields} times");
                }
                return SemaphoreHolder(self);
            }
            match rayon::yield_now() {
                Some(x) => match x {
                    rayon::Yield::Executed => continue,
                    rayon::Yield::Idle => (),
                },
                None => {
                    if panic_if_not_rayon {
                        panic!("not running in a rayon thread pool")
                    }
                }
            }
            os_yields += 1;
            thread::yield_now();
        }
    }

    /// Not efficient, yields to the OS on each failure, and without
    /// blocking!
    pub fn acquire_os(&self) -> SemaphoreHolder<'_> {
        loop {
            if self
                .0
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |old| old.checked_sub(1))
                .is_ok()
            {
                return SemaphoreHolder(self);
            }
            thread::yield_now();
        }
    }
}
