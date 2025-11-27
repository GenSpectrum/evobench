use std::{
    fs::OpenOptions,
    io::Write,
    mem::transmute,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use memmap2::{MmapMut, MmapOptions};

type PollingSignalsAtomic = AtomicU64;

/// A filesystem path based way to poll for 'signals';
pub struct PollingSignals {
    seen: u64,
    mmap: MmapMut,
}

#[derive(thiserror::Error, Debug)]
pub enum PollingSignalsError {
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("invalid length {0} of file contents")]
    InvalidFileContentsLength(u64),
}

impl PollingSignals {
    pub fn open(path: &Path) -> Result<Self, PollingSignalsError> {
        let mut opts = OpenOptions::new();
        opts.read(true);
        opts.write(true);
        opts.truncate(false);
        opts.create(true);
        opts.mode(0o600); // XX how to make portable?
        let mut file = opts.open(path)?;
        let m = file.metadata()?;
        let l = m.len();
        const PSALEN: u64 = size_of::<PollingSignalsAtomic>() as u64;
        match l {
            0 => {
                let a = PollingSignalsAtomic::new(0);
                let b: &[u8; size_of::<PollingSignalsAtomic>()] = unsafe { transmute(&a) };
                file.write_all(b)?;
            }
            PSALEN => (),
            _ => Err(PollingSignalsError::InvalidFileContentsLength(l))?,
        }
        let mmap = unsafe {
            MmapOptions::new()
                .len(size_of::<PollingSignalsAtomic>())
                .map(&file)?
                .make_mut()?
        };
        let mut s = Self { seen: 0, mmap };
        s.get_number_of_signals();
        Ok(s)
    }

    /// Check how many signals were received since the last check.
    pub fn get_number_of_signals(&mut self) -> u64 {
        let Self { seen, mmap } = self;
        let value: &mut [u8; size_of::<PollingSignalsAtomic>()] = (&mut (**mmap)
            [0..size_of::<PollingSignalsAtomic>()])
            .try_into()
            .expect("same size of PollingSignalsAtomic bytes");
        let ptr = value.as_mut_ptr() as *mut AtomicU64;
        let atomic = unsafe { &mut *ptr };
        let new_seen = atomic.load(Ordering::SeqCst);
        let d = new_seen.wrapping_sub(*seen);
        *seen = new_seen;
        d
    }

    /// Send one signal. This is excluded from this `PollingSignals`
    /// instance, i.e. `get_number_of_signals()` will not report it.
    pub fn send_signal(&mut self) -> u64 {
        let Self { seen, mmap } = self;
        *seen = seen.wrapping_add(1);
        let value: &mut [u8; size_of::<PollingSignalsAtomic>()] = (&mut (**mmap)
            [0..size_of::<PollingSignalsAtomic>()])
            .try_into()
            .expect("same size of PollingSignalsAtomic bytes");
        let ptr = value.as_mut_ptr() as *mut AtomicU64;
        let atomic = unsafe { &mut *ptr };
        atomic.fetch_add(1, Ordering::SeqCst)
    }
}
