// Use logging library instead?

use std::{
    io::{BufWriter, StderrLock, Write, stderr},
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
    time::SystemTime,
};

use anyhow::{Result, bail};

use crate::serde::date_and_time::system_time_to_rfc3339;

/// Whether to show time stamps in the local time zone (default: UTC).
pub static LOCAL_TIME: AtomicBool = AtomicBool::new(false);

pub fn write_time(file: &str, line: u32, column: u32) -> BufWriter<StderrLock<'static>> {
    let t = SystemTime::now();
    // Costs an allocation. -- Ordering: probably nobody will be
    // changing it across threads (and if so, ordering probably
    // doesn't matter so much?), thus Relaxed should be fine. Feel
    // free to use Ordering::SeqCst for stores to ensure the last
    // store counts.
    let t_str = system_time_to_rfc3339(t, LOCAL_TIME.load(Ordering::Relaxed));
    let mut lock = BufWriter::new(stderr().lock());
    _ = write!(&mut lock, "{t_str}\t{file}:{line}:{column}\t");
    lock
}

#[macro_export]
macro_rules! info_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = writeln!(&mut lock, $($arg)*);
        }
    }
}

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

    /// Disable warnings. Conflicts with `--verbose` and `--debug`.
    #[clap(short, long)]
    quiet: bool,
}

impl TryFrom<LogLevelOpt> for LogLevel {
    type Error = anyhow::Error;

    fn try_from(value: LogLevelOpt) -> Result<Self> {
        match value {
            LogLevelOpt {
                verbose: false,
                debug: false,
                quiet: false,
            } => Ok(LogLevel::Warn),
            LogLevelOpt {
                verbose: true,
                debug: false,
                quiet: false,
            } => Ok(LogLevel::Info),
            LogLevelOpt {
                verbose: _,
                debug: true,
                quiet: false,
            } => Ok(LogLevel::Debug),
            LogLevelOpt {
                verbose: false,
                debug: false,
                quiet: true,
            } => Ok(LogLevel::Quiet),
            LogLevelOpt {
                verbose: _,
                debug: _,
                quiet: true,
            } => bail!("option `--quiet` conflicts with the options `--verbose` and `--debug`"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Do not log anything
    Quiet,
    /// The default, only "warn!" statements are outputting anythingö.
    Warn,
    /// Verbose execution, not for debugging this program but for
    /// giving the user information about what is going on
    Info,
    /// Highest amount of log statement, for debugging this program
    Debug,
}

impl LogLevel {
    pub const MAX: LogLevel = LogLevel::Debug;

    // Not public api, only for sorting or comparisons!
    fn level(self) -> u8 {
        self as u8
    }

    fn from_level(level: u8) -> Option<Self> {
        let slf = match level {
            0 => Some(LogLevel::Quiet),
            1 => Some(LogLevel::Warn),
            2 => Some(LogLevel::Info),
            3 => Some(LogLevel::Debug),
            _ => None,
        }?;
        assert_eq!(slf.level(), level);
        Some(slf)
    }
}

#[test]
fn t_levels() {
    for i in 0..=LogLevel::MAX.level() {
        let lvl = LogLevel::from_level(i).expect("valid");
        assert_eq!(lvl.level(), i);
    }
    assert_eq!(LogLevel::from_level(LogLevel::MAX.level() + 1), None);
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

pub static LOGLEVEL: AtomicU8 = AtomicU8::new(1);

pub fn set_log_level(val: LogLevel) {
    LOGLEVEL.store(val.level(), Ordering::Relaxed);
}

#[inline]
pub fn log_level() -> LogLevel {
    let level = LOGLEVEL.load(Ordering::Relaxed);
    LogLevel::from_level(level).expect("no possibility to store invalid u8")
}

#[macro_export]
macro_rules! warn {
    { $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Warn {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = writeln!(&mut lock, $($arg)*);
        }
    }
}

#[macro_export]
macro_rules! info {
    { $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Info {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = writeln!(&mut lock, $($arg)*);
        }
    }
}

#[macro_export]
macro_rules! debug {
    { $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Debug {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = writeln!(&mut lock, $($arg)*);
        }
    }
}

// -----------------------------------------------------------------------------

/// Same level as warn, prepending the message with `WARNING: unfinished`
#[macro_export]
macro_rules! unfinished {
    { } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Warn {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = writeln!(&mut lock, "WARNING: unfinished!");
        }
    };
    { $fmt:tt $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Warn {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = lock.write_all("WARNING: unfinished!: ".as_bytes());
            _ = writeln!(&mut lock, $fmt $($arg)*);
        }
    }
}

/// Same level as warn, prepending the message with `WARNING: untested`
#[macro_export]
macro_rules! untested {
    { } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Warn {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = writeln!(&mut lock, "WARNING: untested!");
        }
    };
    { $fmt:tt $($arg:tt)* } => {
        if $crate::utillib::logging::log_level() >= $crate::utillib::logging::LogLevel::Warn {
            use std::io::Write;
            let mut lock = $crate::utillib::logging::write_time(file!(), line!(), column!());
            _ = lock.write_all("WARNING: untested!: ".as_bytes());
            _ = writeln!(&mut lock, $fmt $($arg)*);
        }
    }
}
