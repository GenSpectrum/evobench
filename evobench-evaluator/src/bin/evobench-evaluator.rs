use std::fmt::Display;
use std::fs::{rename, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::index_by_call_path::IndexByCallPath;
use evobench_evaluator::log_data_index::{LogDataIndex, PathStringOptions, SpanId};
use evobench_evaluator::log_file::LogData;
use evobench_evaluator::log_message::Timing;
use evobench_evaluator::path_util::add_extension;
use evobench_evaluator::stats::{Stats, StatsError, ToStatsString};

include!("../../include/evobench_version.rs");

const PROGRAM_NAME: &str = "evobench-evaluator";

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
struct Opts {
    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Print version
    Version,
    /// Read a file
    Read {
        /// Include the internally-allocated thread number in call
        /// path strings in the output.
        #[clap(short, long)]
        show_thread_number: bool,

        /// The path that was provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        path: PathBuf,

        /// Optional path to write CSV output to
        csv_path: Option<PathBuf>,
    },
}

const TILE_COUNT: usize = 101;

fn scopestats<T: Into<u64> + From<u64>>(
    log_data_index: &LogDataIndex,
    spans: &[SpanId],
    extract: impl Fn(&Timing) -> Option<T>,
) -> Result<Stats<T, TILE_COUNT>, StatsError> {
    let vals: Vec<u64> = spans
        .into_iter()
        .filter_map(|span_id| -> Option<u64> {
            let span = span_id.get_from_db(log_data_index);
            let (start, end) = span.start_and_end()?;
            Some(extract(end)?.into() - extract(start)?.into())
        })
        .collect();
    Stats::from_values(vals)
}

fn stats<T: Into<u64> + From<u64> + ToStatsString + Display>(
    log_data_index: &LogDataIndex,
    spans: &[SpanId],
    extract_name: &str,
    pn: &str,
    extract: impl Fn(&Timing) -> Option<T>,
    mut out: impl Write,
) -> Result<()> {
    let r: Result<Stats<T, TILE_COUNT>, StatsError> = scopestats(log_data_index, spans, extract);
    match r {
        Ok(s) => {
            // eprintln!("{pn:?} => {s}");
            s.print_tsv_line(&mut out, &[extract_name, pn])?;
        }
        Err(StatsError::NoInputs) => {
            // XX more generic? print_tsv_line directly on Result? Or evil
            // anyway? Actually showing counts here now, evil too.
            let count = spans.len();
            writeln!(&mut out, "{extract_name}\t{pn}\t{count}")?;
        }
        Err(e) => Err(e)?,
    }
    Ok(())
}

fn stats_all_probes<T: Into<u64> + From<u64> + ToStatsString + Display>(
    mut out: impl Write,
    log_data_index: &LogDataIndex,
    index_by_call_path: &IndexByCallPath,
    extract_name: &str,
    extract: impl Fn(&Timing) -> Option<T>,
) -> Result<()> {
    // eprintln!("----{extract_name}-----------------------------------------------------------------------------------");

    // Separate the tables from each other in the TSV
    writeln!(&mut out, "")?;
    Stats::<T, TILE_COUNT>::print_tsv_header(&mut out, &["field", "probe name"])?;
    for pn in log_data_index.probe_names() {
        stats(
            log_data_index,
            log_data_index.spans_by_pn(&pn).unwrap(),
            extract_name,
            pn,
            &extract,
            &mut out,
        )?;
    }

    for call_path in index_by_call_path.call_paths() {
        stats(
            log_data_index,
            index_by_call_path.spans_by_call_path(call_path).unwrap(),
            extract_name,
            call_path,
            &extract,
            &mut out,
        )?;
    }

    Ok(())
}

fn main() -> Result<()> {
    let opts: Opts = Opts::parse();
    match &opts.command {
        Command::Version => println!("{PROGRAM_NAME} version {EVOBENCH_VERSION}"),
        Command::Read {
            path,
            show_thread_number,
            csv_path,
        } => {
            let data = LogData::read_file(path, None)?;
            let log_data_index = LogDataIndex::from_logdata(&data)?;

            let index_by_call_path = {
                // Note: it's important to give prefixes here, to
                // avoid getting rows that have the scopes counted
                // *twice* (currently just "main thread"). (Could
                // handle that in `IndexByCallPath::from_logdataindex`
                // (by using a set instead of Vec), but having 1 entry
                // that only counts thing once, but is valid for both
                // kinds of groups, would surely still be confusing.)
                let mut opts = vec![PathStringOptions {
                    ignore_process: true,
                    ignore_thread: true,
                    include_thread_number_in_path: false,
                    // "across threads / added up"
                    prefix: "A:",
                }];
                if *show_thread_number {
                    opts.push(PathStringOptions {
                        ignore_process: true,
                        ignore_thread: true,
                        include_thread_number_in_path: true,
                        // "numbered threads"
                        prefix: "N:",
                    });
                }
                IndexByCallPath::from_logdataindex(&log_data_index, &opts)
            };

            if let Some(csv_path) = csv_path {
                let csv_path_tmp = add_extension(csv_path, "tmp")
                    .ok_or_else(|| anyhow!("path misses a filename: {csv_path:?}"))?;
                let mut out =
                    BufWriter::new(File::create(&csv_path_tmp).with_context(|| {
                        anyhow!("can't open file for writing: {csv_path_tmp:?}")
                    })?);

                (|| -> Result<()> {
                    stats_all_probes(
                        &mut out,
                        &log_data_index,
                        &index_by_call_path,
                        "real time",
                        |timing: &Timing| Some(timing.r),
                    )?;
                    stats_all_probes(
                        &mut out,
                        &log_data_index,
                        &index_by_call_path,
                        "cpu time",
                        |timing: &Timing| Some(timing.u),
                    )?;
                    stats_all_probes(
                        &mut out,
                        &log_data_index,
                        &index_by_call_path,
                        "sys time",
                        |timing: &Timing| Some(timing.s),
                    )?;
                    stats_all_probes(
                        &mut out,
                        &log_data_index,
                        &index_by_call_path,
                        "ctx switches",
                        |timing: &Timing| Some(timing.nvcsw()? + timing.nivcsw()?),
                    )?;

                    out.flush()?;

                    Ok(())
                })()
                .with_context(|| anyhow!("writing output to file {csv_path_tmp:?}"))?;

                rename(&csv_path_tmp, csv_path)
                    .with_context(|| anyhow!("renaming {csv_path_tmp:?} to {csv_path:?}"))?;
            } else {
                println!("OK, but not printing. Please give a CSV output path!");
            }
        }
    }

    Ok(())
}
