// Use logging library instead?

use std::{
    io::{BufWriter, StderrLock, Write, stderr},
    str::FromStr,
    sync::atomic::{AtomicU8, Ordering},
};

use anyhow::{Result, bail};
use strum::VariantNames;
use strum_macros::{EnumVariantNames, ToString};

use crate::serde::date_and_time::DateTimeWithOffset;

pub fn write_time(file: &str, line: u32, column: u32) -> BufWriter<StderrLock<'static>> {
    // Costs an allocation.
    let t_str = DateTimeWithOffset::now(None);
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

// Do *not* make the fields public here to force going through
// `From`/`Into`, OK? Also, do not add Clone to force evaluation
// before doing anything further, OK?
#[derive(Debug, clap::Args)]
pub struct LogLevelOpts {
    /// Disable warnings. Conflicts with `--verbose` and `--debug`
    /// (decreases log-level from 'warn' to 'quiet'--only errors
    /// interrupting processing are shown)
    #[clap(short, long)]
    quiet: bool,

    /// Show what is being done (increases log-level from 'warn' to
    /// 'info')
    #[clap(short, long)]
    verbose: bool,

    /// Show information that helps debug this program (implies
    /// `--verbose`) (increases log-level from 'warn' to 'debug')
    #[clap(short, long)]
    debug: bool,
}

impl LogLevelOpts {
    /// Complain if both options in self and `opt_log_level` are
    /// given.
    pub fn xor_log_level(self, opt_log_level: Option<LogLevel>) -> Result<LogLevel> {
        if let Some(level) = TryInto::<Option<LogLevel>>::try_into(self)? {
            if opt_log_level.is_some() {
                bail!(
                    "both the {} option and a log-level were given, please \
                     only either give one of the options --quiet / --verbose / --debug \
                     or a log-level",
                    level
                        .option_name()
                        .expect("if TryInto gave a value then option_name will give one, too")
                )
            }
            Ok(level)
        } else {
            Ok(opt_log_level.unwrap_or_default())
        }
    }
}

impl TryFrom<LogLevelOpts> for LogLevel {
    type Error = anyhow::Error;

    fn try_from(value: LogLevelOpts) -> Result<Self> {
        match value {
            LogLevelOpts {
                verbose: false,
                debug: false,
                quiet: false,
            } => Ok(LogLevel::Warn),
            LogLevelOpts {
                verbose: true,
                debug: false,
                quiet: false,
            } => Ok(LogLevel::Info),
            LogLevelOpts {
                verbose: _,
                debug: true,
                quiet: false,
            } => Ok(LogLevel::Debug),
            LogLevelOpts {
                verbose: false,
                debug: false,
                quiet: true,
            } => Ok(LogLevel::Quiet),
            LogLevelOpts {
                verbose: _,
                debug: _,
                quiet: true,
            } => bail!("option `--quiet` conflicts with the options `--verbose` and `--debug`"),
        }
    }
}

/// Like `TryFrom<LogLevelOpts> for LogLevel` but returns None if none
/// of the 3 options were given.
impl TryFrom<LogLevelOpts> for Option<LogLevel> {
    type Error = anyhow::Error;

    fn try_from(value: LogLevelOpts) -> std::result::Result<Self, Self::Error> {
        match value {
            LogLevelOpts {
                verbose: false,
                debug: false,
                quiet: false,
            } => Ok(None),
            _ => Ok(Some(value.try_into()?)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ToString, EnumVariantNames)]
#[strum(serialize_all = "snake_case")]
pub enum LogLevel {
    /// Do not log anything
    Quiet,
    /// The default, only "warn!" statements are outputting anything.
    Warn,
    /// Verbose execution, not for debugging this program but for
    /// giving the user information about what is going on
    Info,
    /// Highest amount of log statement, for debugging this program
    Debug,
}

impl LogLevel {
    pub const MAX: LogLevel = LogLevel::Debug;

    // Not public api, only for sorting or comparisons! / level
    // setting API.

    fn level(self) -> u8 {
        self as u8
    }

    fn from_level(level: u8) -> Option<Self> {
        {
            // Reminder
            match LogLevel::Quiet {
                LogLevel::Quiet => (),
                LogLevel::Warn => (),
                LogLevel::Info => (),
                LogLevel::Debug => (),
            }
        }
        match level {
            0 => Some(LogLevel::Quiet),
            1 => Some(LogLevel::Warn),
            2 => Some(LogLevel::Info),
            3 => Some(LogLevel::Debug),
            _ => None,
        }
    }

    /// The name of the (predominant) option that yields this
    /// log-level
    fn option_name(self) -> Option<&'static str> {
        match self {
            LogLevel::Quiet => Some("--quiet"),
            LogLevel::Warn => None,
            LogLevel::Info => Some("--verbose"),
            LogLevel::Debug => Some("--debug"),
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Warn
    }
}

#[test]
fn t_default() {
    assert_eq!(
        LogLevel::default(),
        LogLevelOpts {
            verbose: false,
            debug: false,
            quiet: false
        }
        .try_into()
        .expect("no conflicts")
    );
    assert_eq!(LogLevel::default().option_name(), None);
}

// Sigh, strum_macros::EnumString is useless as it does not show the
// variants in its error message. (Clap 4 has its own macro instead?)
impl FromStr for LogLevel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        {
            // Reminder
            match LogLevel::Quiet {
                LogLevel::Quiet => (),
                LogLevel::Warn => (),
                LogLevel::Info => (),
                LogLevel::Debug => (),
            }
        }
        match s {
            "quiet" => Ok(LogLevel::Quiet),
            "warn" => Ok(LogLevel::Warn),
            "info" => Ok(LogLevel::Info),
            "debug" => Ok(LogLevel::Debug),
            _ => bail!(
                "invalid log level name {s:?}, valid are: {}",
                LogLevel::VARIANTS.join(", ")
            ),
        }
    }
}

#[test]
fn t_levels() -> Result<()> {
    use std::str::FromStr;

    for i in 0..=LogLevel::MAX.level() {
        let lvl = LogLevel::from_level(i).expect("valid");
        assert_eq!(lvl.level(), i);
        let s = lvl.to_string();
        assert_eq!(LogLevel::from_str(&s).unwrap(), lvl);
    }
    assert_eq!(LogLevel::from_level(LogLevel::MAX.level() + 1), None);
    let lvl = LogLevel::from_str("info")?;
    assert_eq!(lvl.level(), 2);
    assert!(LogLevel::from_str("Info").is_err());
    assert_eq!(lvl.to_string(), "info");
    Ok(())
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

pub static LOG_LEVEL: AtomicU8 = AtomicU8::new(1);

pub fn set_log_level(val: LogLevel) {
    LOG_LEVEL.store(val.level(), Ordering::SeqCst);
}

#[inline]
pub fn log_level() -> LogLevel {
    let level = LOG_LEVEL.load(Ordering::Relaxed);
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
