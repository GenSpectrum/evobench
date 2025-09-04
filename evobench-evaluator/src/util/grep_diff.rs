use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, FixedOffset};
use itertools::Itertools;
use regex::Regex;

use crate::{
    ctx,
    git::GitHash,
    info,
    key::{BenchmarkingJobParameters, RunParameters},
    run::run_job::AllowableCustomEnvVar,
    serde::{allowed_env_var::AllowedEnvVar, proper_dirname::ProperDirname},
    warn,
};

// XX move to pair up with code for writing log files!
fn parse_log_file_params(s: &str) -> Result<Option<(BenchmarkingJobParameters, &str)>> {
    // Should have added a separator to the files (now it outputs an
    // empty line, but have to deal with older files, too): scan until
    // finding the first timestamp, then assume the part before is the
    // head.
    let line_endings = s.char_indices().filter(|(_, c)| *c == '\n');
    for (i, _) in line_endings {
        let rest = &s[i + 1..];
        if let Some((t, _)) = rest.split_once('\t') {
            if let Ok(_timestamp) = DateTime::parse_from_rfc3339(t) {
                let head = &s[0..i];
                return Ok(Some((serde_yml::from_str(head)?, rest)));
            }
        }
    }
    Ok(None)
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

pub fn grep_diff(
    regex_start: String,
    regex_end: String,
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

    let regex_start =
        Regex::new(&regex_start).map_err(ctx!("compiling start regex {regex_start:?}"))?;
    let regex_end = Regex::new(&regex_end).map_err(ctx!("compiling end regex {regex_end:?}"))?;

    'logfile: for logfile in &logfiles {
        let log_contents = std::fs::read_to_string(logfile).map_err(ctx!("f"))?;

        let (
            BenchmarkingJobParameters {
                run_parameters,
                command,
            },
            log_contents_rest,
        ) = if let Some(params) = parse_log_file_params(&log_contents)
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

        // Extract the time span
        match grep_diff_str(&regex_start, &regex_end, log_contents_rest) {
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

                    println!("{s}.{ns}\t{commit_id}\t{target_name}\t{custom_parameters}\t{logfile_str}:{}", start.lineno0 + 1);
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
