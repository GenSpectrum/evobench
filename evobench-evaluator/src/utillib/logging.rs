use std::sync::atomic::{AtomicBool, Ordering};

#[macro_export]
macro_rules! info_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprintln!($($arg)*);
        }
    }
}

#[macro_export]
macro_rules! info_noln_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprint!($($arg)*);
        }
    }
}

// TODO: use logging library

pub static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(val: bool) {
    VERBOSE.store(val, Ordering::Relaxed);
}

#[inline]
pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

#[macro_export]
macro_rules! info {
    { $($arg:tt)* } => {
        if $crate::utillib::logging::verbose() {
            eprintln!($($arg)*);
        }
    }
}
