use std::sync::atomic::{AtomicU8, Ordering};

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

// TODO: use logging library?

// Do *not* make the fields public here to force going through `From`/`Into`, OK?
#[derive(Debug, clap::Args)]
pub struct LogLevelOpt {
    /// Show what is being done
    #[clap(short, long)]
    verbose: bool,

    /// Show information that helps debug this program (implies
    /// `--verbose`)
    #[clap(short, long)]
    debug: bool,
}

impl From<LogLevelOpt> for LogLevel {
    fn from(value: LogLevelOpt) -> Self {
        match value {
            LogLevelOpt {
                verbose: false,
                debug: false,
            } => LogLevel::None,
            LogLevelOpt {
                verbose: true,
                debug: false,
            } => LogLevel::Info,
            LogLevelOpt {
                verbose: _,
                debug: true,
            } => LogLevel::Debug,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Do not log anything
    None,
    /// Verbose execution, not for debugging this program but for
    /// giving the user information about what is going on
    Info,
    /// Highest amount of log statement, for debugging this program
    Debug,
}

impl LogLevel {
    // Not public api, only for sorting or comparisons!
    fn level(self) -> u8 {
        self as u8
    }

    fn from_level(level: u8) -> Option<Self> {
        let slf = match level {
            0 => Some(LogLevel::None),
            1 => Some(LogLevel::Info),
            2 => Some(LogLevel::Debug),
            _ => None,
        }?;
        assert_eq!(slf.level(), level);
        Some(slf)
    }
}

#[test]
fn t_levels() {
    for i in 0..=2 {
        _ = LogLevel::from_level(i);
    }
}

impl PartialOrd for LogLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.level().cmp(&other.level())
    }
}

pub static LOGLEVEL: AtomicU8 = AtomicU8::new(0);

pub fn set_log_level(val: LogLevel) {
    LOGLEVEL.store(val.level(), Ordering::Relaxed);
}

#[inline]
pub fn log_level() -> LogLevel {
    let level = LOGLEVEL.load(Ordering::Relaxed);
    LogLevel::from_level(level).expect("no possibility to store invalid u8")
}

#[macro_export]
macro_rules! info {
    { $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Info {
            eprintln!($($arg)*);
        }
    }
}

#[macro_export]
macro_rules! debug {
    { $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Debug {
            eprintln!($($arg)*);
        }
    }
}
