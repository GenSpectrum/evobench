use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, FixedOffset};
use clap::Parser;

use evobench_evaluator::{
    ctx,
    get_terminal_width::get_terminal_width,
    utillib::logging::{set_log_level, LogLevelOpt},
    warn,
};
use itertools::Itertools;
use regex::Regex;

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Utilities for working with evobench
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// Extract time differences between pairs of lines in log files
    /// from benchmarking runs--not the evobench.log files, but files
    /// with captured stdout/stderr, in the working directory pool
    /// directory (like `$n.output_of_benchmarking_command_at_*`).
    GrepDiff {
        /// The regex to match a log lie that starts a timed region
        regex_start: String,

        /// The regex to match a log lie that ends a timed region
        regex_end: String,

        /// Override the path to the config file (default: the paths
        /// `~/.evobench-run.*` where a single one exists where the `*` is
        /// the suffix for one of the supported config file formats (run
        /// `config-formats` to get the list), and if those are missing,
        /// use compiled-in default config values)
        logfiles: Vec<PathBuf>,
    },
}

// != evaluator::data::log_data::Timing
struct Timing {
    timestamp: DateTime<FixedOffset>,
    lineno0: usize,
    rest: String,
}

// != evaluator::data::log_data_tree::Span
struct Span {
    start: Timing,
    end: Timing,
}

impl Span {
    fn duration(&self) -> chrono::Duration {
        let Self { start, end } = self;
        end.timestamp.signed_duration_since(start.timestamp)
    }
}

/// Returns the start-`Timing` as an error if no match for `regex_end`
/// was found after it.
fn grep_diff_str(
    regex_start: &Regex,
    regex_end: &Regex,
    log_contents: &str,
) -> Result<Vec<Span>, Timing> {
    let mut spans = Vec::new();
    let mut lines = log_contents.split('\n').enumerate();
    while let Some((lineno0, line)) = lines.next() {
        if let Some((t, rest)) = line.split_once('\t') {
            if regex_start.is_match(rest) {
                if let Ok(timestamp) = DateTime::parse_from_rfc3339(t) {
                    let start = Timing {
                        timestamp,
                        lineno0,
                        rest: rest.into(),
                    };
                    'inner: {
                        while let Some((lineno0, line)) = lines.next() {
                            if let Some((t, rest)) = line.split_once('\t') {
                                if regex_end.is_match(rest) {
                                    if let Ok(timestamp) = DateTime::parse_from_rfc3339(t) {
                                        let end = Timing {
                                            timestamp,
                                            lineno0,
                                            rest: rest.into(),
                                        };
                                        spans.push(Span { start, end });
                                        break 'inner;
                                    }
                                }
                            }
                        }
                        return Err(start);
                    }
                }
            }
        }
    }
    Ok(spans)
}

fn grep_diff(regex_start: String, regex_end: String, logfiles: Vec<PathBuf>) -> Result<()> {
    let regex_start =
        Regex::new(&regex_start).map_err(ctx!("compiling start regex {regex_start:?}"))?;
    let regex_end = Regex::new(&regex_end).map_err(ctx!("compiling end regex {regex_end:?}"))?;

    for logfile in &logfiles {
        let log_contents = std::fs::read_to_string(logfile).map_err(ctx!("f"))?;
        match grep_diff_str(&regex_start, &regex_end, &log_contents) {
            Ok(spans) => match spans.len() {
                0 => {
                    warn!("file {logfile:?} has no match");
                }
                1 => {
                    let span = &spans[0];
                    let Span { start, end } = span;
                    let logfile_str = logfile.to_string_lossy();
                    let duration = span.duration();
                    let ns = duration.num_nanoseconds().ok_or_else(|| {
                        let logfile_str = logfile.to_string_lossy();
                        anyhow!(
                            "file {logfile_str}:{} to {}: time span does not fit u64 nanoseconds",
                            start.lineno0 + 1,
                            end.lineno0 + 1
                        )
                    })?;
                    let s = ns / 1_000_000_000;
                    let ns = ns % 1_000_000_000;
                    println!("{s}.{ns}\t{logfile_str}:{}", start.lineno0 + 1);
                }
                _ => {
                    let msg = spans
                        .iter()
                        .map(|Span { start, end }| {
                            format!("lines {} to {}", start.lineno0 + 1, end.lineno0 + 1)
                        })
                        .join(", ");
                    bail!("file {logfile:?} has more than one match: {msg}");
                }
            },
            Err(Timing {
                timestamp,
                lineno0,
                rest,
            }) => {
                let logfile_str = logfile.to_string_lossy();
                warn!("file {logfile_str}:{} matches start but no end match after: {timestamp}\t{rest}",
                      lineno0 + 1);
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let Opts {
        log_level,
        subcommand,
    } = Opts::parse();

    set_log_level(log_level.try_into()?);

    match subcommand {
        SubCommand::GrepDiff {
            regex_start,
            regex_end,
            logfiles,
        } => grep_diff(regex_start, regex_end, logfiles)?,
    }

    Ok(())
}
