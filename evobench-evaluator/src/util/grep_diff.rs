use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, FixedOffset};
use itertools::Itertools;
use regex::Regex;

use crate::{
    ctx,
    git::GitHash,
    info,
    key::{BenchmarkingJobParameters, RunParameters},
    run::{
        command_log_file::{CommandLog, CommandLogFile},
        run_job::AllowableCustomEnvVar,
    },
    serde::{allowed_env_var::AllowedEnvVar, proper_dirname::ProperDirname},
    times::{NanoTime, ToStringSeconds},
    warn,
};

// != evaluator::data::log_data::Timing
pub struct Timing {
    timestamp: DateTime<FixedOffset>,
    lineno: usize,
    rest: String,
}

// != evaluator::data::log_data_tree::Span
pub struct Span {
    start: Timing,
    end: Timing,
}

impl Span {
    /// Could be negative for invalid logfiles
    fn duration(&self) -> chrono::Duration {
        let Self { start, end } = self;
        end.timestamp.signed_duration_since(start.timestamp)
    }

    /// Returns errors for durations that are negative or too large
    fn duration_nanotime(&self) -> Result<NanoTime> {
        let duration = self.duration();
        let ns: i64 = duration
            .num_nanoseconds()
            .ok_or_else(|| anyhow!("time span does not fit u64 nanoseconds"))?;
        let ns =
            u64::try_from(ns).map_err(ctx!("trying to convert duration to unsigned number"))?;

        NanoTime::from_nsec(ns).ok_or_else(|| anyhow!("duration too large to fit NanoTime"))
    }
}

pub struct GrepDiffRegion {
    pub regex_start: Regex,
    pub regex_end: Regex,
}

impl GrepDiffRegion {
    pub fn from_strings(regex_start: &str, regex_end: &str) -> Result<Self> {
        let (regex_start, regex_end) = (
            Regex::new(regex_start).map_err(ctx!("compiling start regex {regex_start:?}"))?,
            Regex::new(regex_end).map_err(ctx!("compiling end regex {regex_end:?}"))?,
        );
        Ok(Self {
            regex_start,
            regex_end,
        })
    }

    /// Returns the start-`Timing` as an error if no match for `regex_end`
    /// was found after it.
    pub fn find_matching_spans_for(
        &self,
        (log_contents, lineno): (&str, usize),
    ) -> Result<Vec<Span>, Timing> {
        let mut spans = Vec::new();
        let mut lines = log_contents.split('\n').enumerate();
        while let Some((lineno0, line)) = lines.next() {
            if let Some((t, rest)) = line.split_once('\t') {
                if self.regex_start.is_match(rest) {
                    if let Ok(timestamp) = DateTime::parse_from_rfc3339(t) {
                        let start = Timing {
                            timestamp,
                            lineno: lineno + lineno0,
                            rest: rest.into(),
                        };
                        'inner: {
                            while let Some((lineno0, line)) = lines.next() {
                                if let Some((t, rest)) = line.split_once('\t') {
                                    if self.regex_end.is_match(rest) {
                                        if let Ok(timestamp) = DateTime::parse_from_rfc3339(t) {
                                            let end = Timing {
                                                timestamp,
                                                lineno: lineno + lineno0,
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

    // Extract the single expected time span
    pub fn find_duration_for<P: AsRef<Path>>(
        &self,
        command_log: &CommandLog<P>,
    ) -> Result<Option<Span>> {
        match self.find_matching_spans_for(command_log.log_contents_rest()) {
            Ok(mut spans) => match spans.len() {
                0 => {
                    warn!("file {:?} has no match", command_log.path());
                    Ok(None)
                }
                1 => Ok(spans.pop()),
                _ => {
                    let msg = spans
                        .iter()
                        .map(|Span { start, end }| {
                            format!("lines {} to {}", start.lineno, end.lineno)
                        })
                        .join(", ");
                    bail!(
                        "file {:?} has more than one match: {msg}",
                        command_log.path()
                    );
                }
            },
            Err(Timing {
                timestamp,
                lineno,
                rest,
            }) => {
                warn!(
                    "file {}:{} matches start but no end thereafter: {timestamp}\t{rest}",
                    command_log.path_string_lossy(),
                    lineno
                );
                Ok(None)
            }
        }
    }

    pub fn grep_diff(
        &self,
        logfiles: Vec<PathBuf>,
        commit_filter: Option<GitHash>,
        target_name_filter: Option<ProperDirname>,
        params_filter: Option<String>,
    ) -> Result<()> {
        let params_filter = if let Some(params_filter) = &params_filter {
            let mut keyvals = Vec::new();
            for keyval in params_filter.split('/') {
                let (key, val) = keyval.split_once('=').ok_or_else(|| {
                    anyhow!("missing '=' in variable key-value pair string {keyval:?}")
                })?;
                let key: AllowedEnvVar<AllowableCustomEnvVar> = key.parse()?;
                keyvals.push((key, val));
            }
            keyvals
        } else {
            Vec::new()
        };

        'logfile: for logfile in &logfiles {
            let command_log_file = CommandLogFile { path: logfile };
            let command_log = command_log_file.command_log()?;
            let BenchmarkingJobParameters {
                run_parameters,
                command,
            } = if let Some(params) = command_log
                .parse_log_file_params()
                .map_err(ctx!("can't parse header of log file {logfile:?}"))?
            {
                params
            } else {
                warn!("file {logfile:?} has no log file info header, skipping");
                continue 'logfile;
            };
            #[allow(unused)]
            let log_contents = ();

            let RunParameters {
                commit_id,
                custom_parameters,
            } = &*run_parameters;
            let target_name = &command.target_name;

            // Filter according to given filtering options

            if let Some(commit) = &commit_filter {
                if commit != commit_id {
                    info!("file {logfile:?} does not match commit");
                    continue 'logfile;
                }
            }

            if let Some(target_name_filter) = &target_name_filter {
                if target_name_filter != target_name {
                    info!("file {logfile:?} does not match target name");
                    continue 'logfile;
                }
            }

            for (key, val) in &params_filter {
                if let Some(log_val) = custom_parameters.btree_map().get(key) {
                    if *val != log_val.as_str() {
                        info!("file {logfile:?} does not match custom variable '{key}'='{val}'");
                        continue 'logfile;
                    }
                } else {
                    info!("file {logfile:?} does not use custom variable '{key}'");
                    continue 'logfile;
                }
            }

            if let Some(span) = self.find_duration_for(&command_log)? {
                let duration = span.duration_nanotime().map_err(ctx!(
                    "file {}:{} to {}",
                    command_log.path_string_lossy(),
                    span.start.lineno,
                    span.end.lineno
                ))?;

                let logfile_str = logfile.to_string_lossy();
                println!(
                    "{}\t{commit_id}\t{target_name}\t{custom_parameters}\t{logfile_str}:{}",
                    duration.to_string_seconds(),
                    span.start.lineno
                );
            }
        }

        Ok(())
    }
}
