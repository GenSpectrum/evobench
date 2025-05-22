use std::borrow::Cow;
use std::fmt::{Debug, Display};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use evobench_evaluator::excel_table_view::excel_file_write;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::index_by_call_path::IndexByCallPath;
use evobench_evaluator::io_util::xrename;
use evobench_evaluator::log_data_index::{LogDataIndex, PathStringOptions, SpanId};
use evobench_evaluator::log_file::LogData;
use evobench_evaluator::log_message::Timing;
use evobench_evaluator::path_util::add_extension;
use evobench_evaluator::stats::{Stats, StatsError, ToStatsString};
use evobench_evaluator::table::{KeyVal, StatsOrCount, Table, TableKeyLabel};
use evobench_evaluator::table_view::TableView;

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

    /// Show statistics for a single benchmarking log file
    Read {
        /// The width of the column with the probes path, in characters
        /// (as per Excel's definition of characters)
        #[clap(short, long, default_value = "100")]
        key_width: f64,

        /// Path to write CSV output to
        #[clap(short, long)]
        csv: Option<PathBuf>,

        /// Path to write Excel output to
        #[clap(short, long)]
        excel: Option<PathBuf>,

        /// Include the internally-allocated thread number in call
        /// path strings in the output.
        #[clap(short, long)]
        show_thread_number: bool,

        /// The path that was provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        path: PathBuf,
    },
}

// We use 101 buckets for percentiles instead of 100, so that we get
// buckets at positions 50, 25, 75 for exact matches, OK? (Although
// note that the `Stats` median is not based on those buckets
// (anymore).)
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

fn pn_stats<'t, T: Into<u64> + From<u64> + ToStatsString + Display + Debug>(
    log_data_index: &LogDataIndex,
    spans: &[SpanId],
    pn: &'t str,
    extract: impl Fn(&Timing) -> Option<T>,
) -> Result<KeyVal<Cow<'t, str>, StatsOrCount<T, TILE_COUNT>>, StatsError> {
    let r: Result<Stats<T, TILE_COUNT>, StatsError> = scopestats(log_data_index, spans, extract);
    match r {
        Ok(s) => Ok(KeyVal {
            key: pn.into(),
            val: StatsOrCount::Stats(s),
        }),
        Err(StatsError::NoInputs) => {
            let count = spans.len();
            Ok(KeyVal {
                key: pn.into(),
                val: StatsOrCount::Count(count),
            })
        }
        Err(e) => Err(e),
    }
}

pub struct PathLabel;
impl TableKeyLabel for PathLabel {
    const KEY_LABEL: &str = "Probe name or path\n(A: across all threads, N: by thread number)";
}

/// A table holding one field for all probes
fn table_for_field<'t, T: Into<u64> + From<u64> + ToStatsString + Display + Debug>(
    extract_name: &'t str,
    extract: impl Fn(&Timing) -> Option<T>,
    log_data_index: &'t LogDataIndex,
    index_by_call_path: &'t IndexByCallPath,
    key_column_width: f64,
) -> Result<Table<'t, StatsOrCount<T, TILE_COUNT>, PathLabel>> {
    let mut rows = Vec::new();
    for pn in log_data_index.probe_names() {
        rows.push(pn_stats(
            log_data_index,
            log_data_index.spans_by_pn(&pn).unwrap(),
            pn,
            &extract,
        )?);
    }

    for call_path in index_by_call_path.call_paths() {
        rows.push(pn_stats(
            log_data_index,
            index_by_call_path.spans_by_call_path(call_path).unwrap(),
            call_path,
            &extract,
        )?);
    }

    Ok(Table {
        key_label: Default::default(),
        name: extract_name.into(),
        rows,
        key_column_width: Some(key_column_width),
    })
}

fn main() -> Result<()> {
    let Opts { command } = Opts::parse();
    match command {
        Command::Version => println!("{PROGRAM_NAME} version {EVOBENCH_VERSION}"),
        Command::Read {
            key_width,
            path,
            show_thread_number,
            csv,
            excel,
        } => {
            let data = LogData::read_file(&path, None)?;
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
                if show_thread_number {
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

            if let Some(csv_path) = csv {
                let csv_path_tmp = add_extension(&csv_path, "tmp")
                    .ok_or_else(|| anyhow!("path misses a filename: {csv_path:?}"))?;
                let mut _out =
                    BufWriter::new(File::create(&csv_path_tmp).with_context(|| {
                        anyhow!("can't open file for writing: {csv_path_tmp:?}")
                    })?);
                bail!("TODO re-add code for CSV writing?");
                // out.flush()?;
                // xrename(&csv_path_tmp, csv_path)?;
            }

            if let Some(excel_path) = excel {
                let path_tmp = add_extension(&excel_path, "tmp")
                    .ok_or_else(|| anyhow!("path misses a filename: {excel_path:?}"))?;

                (|| -> Result<()> {
                    let mut tables: Vec<Box<dyn TableView>> = vec![];
                    tables.push(Box::new(table_for_field(
                        "real time",
                        |timing: &Timing| Some(timing.r),
                        &log_data_index,
                        &index_by_call_path,
                        key_width,
                    )?));

                    tables.push(Box::new(table_for_field(
                        "cpu time",
                        |timing: &Timing| Some(timing.u),
                        &log_data_index,
                        &index_by_call_path,
                        key_width,
                    )?));
                    tables.push(Box::new(table_for_field(
                        "sys time",
                        |timing: &Timing| Some(timing.s),
                        &log_data_index,
                        &index_by_call_path,
                        key_width,
                    )?));
                    tables.push(Box::new(table_for_field(
                        "ctx switches",
                        |timing: &Timing| Some(timing.nvcsw()? + timing.nivcsw()?),
                        &log_data_index,
                        &index_by_call_path,
                        key_width,
                    )?));

                    excel_file_write(&tables, &path_tmp)?;
                    xrename(&path_tmp, &excel_path)?;

                    Ok(())
                })()
                .with_context(|| anyhow!("writing output to file {excel_path:?}"))?;
            } else {
                println!("OK, but not printing. Please give a CSV output path!");
            }
        }
    }

    Ok(())
}
