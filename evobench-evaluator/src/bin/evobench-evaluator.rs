use std::fmt::Debug;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::evaluator::evaluator::{
    AllFieldsTable, AllFieldsTableKindParams, KeyRuntimeDetails, SingleRunStats, SummaryStats,
};
use evobench_evaluator::evaluator::options::{
    EvaluationOpts, FieldSelectorDimension3, FieldSelectorDimension4,
};
use evobench_evaluator::excel_table_view::excel_file_write;
use evobench_evaluator::get_terminal_width::get_terminal_width;
use evobench_evaluator::log_data_and_tree::LogDataAndTree;
use mimalloc::MiMalloc;
use rayon::iter::IntoParallelRefIterator;
use rayon::prelude::ParallelIterator;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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
    Single {
        #[clap(flatten)]
        evaluation_opts: EvaluationOpts,

        /// The path that was provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        path: PathBuf,
    },

    /// Show statistics for a set of benchmarking log files, all for
    /// the same software version.
    Summary {
        #[clap(flatten)]
        evaluation_opts: EvaluationOpts,
        #[clap(flatten)]
        field_selector_dimension_3: FieldSelectorDimension3,

        /// The paths that were provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        paths: Vec<PathBuf>,
    },

    /// Show statistics across multiple sets of benchmarking log
    /// files, each group consisting of files for the same software
    /// version. Each group is enclosed with square brackets, e.g.:
    /// `trend [ a.log b.log ] [ c.log ] [ d.log e.log ]` has data for
    /// 3 software versions, the first and third version with data
    /// from two runs each.
    Trend {
        #[clap(flatten)]
        evaluation_opts: EvaluationOpts,
        #[clap(flatten)]
        field_selector_dimension_3: FieldSelectorDimension3,
        #[clap(flatten)]
        field_selector_dimension_4: FieldSelectorDimension4,

        /// The paths that were provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        grouped_paths: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let Opts { command } = Opts::parse();
    match command {
        Command::Version => println!("{PROGRAM_NAME} version {EVOBENCH_VERSION}"),

        Command::Single {
            evaluation_opts:
                EvaluationOpts {
                    key_width,
                    excel,
                    show_thread_number,
                    show_reversed,
                },
            path,
        } => {
            let ldat = LogDataAndTree::read_file(&path, None)?;
            let aft = AllFieldsTable::from_log_data_tree(
                ldat.tree(),
                AllFieldsTableKindParams {
                    path,
                    key_width,
                    key_details: KeyRuntimeDetails {
                        show_thread_number,
                        key_column_width: Some(key_width),
                        show_reversed,
                    },
                },
            )?;
            excel_file_write(&aft.tables(), &excel)?;
        }

        Command::Summary {
            evaluation_opts:
                EvaluationOpts {
                    key_width,
                    excel,
                    show_thread_number,
                    show_reversed,
                },
            paths,
            field_selector_dimension_3: FieldSelectorDimension3 { summary_field },
        } => {
            let afts: Vec<AllFieldsTable<SingleRunStats>> = paths
                .par_iter()
                .map(|path| {
                    let ldat = LogDataAndTree::read_file(path, None)?;
                    AllFieldsTable::from_log_data_tree(
                        ldat.tree(),
                        AllFieldsTableKindParams {
                            path: path.into(),
                            key_width,
                            key_details: KeyRuntimeDetails {
                                show_thread_number,
                                key_column_width: Some(key_width),
                                show_reversed,
                            },
                        },
                    )
                })
                .collect::<Result<_>>()?;
            let aft = AllFieldsTable::<SummaryStats>::summary_stats(
                summary_field,
                &KeyRuntimeDetails {
                    show_thread_number,
                    key_column_width: Some(key_width),
                    show_reversed,
                },
                &afts,
            );
            excel_file_write(&aft.tables(), &excel)?;
        }

        Command::Trend {
            evaluation_opts,
            grouped_paths,
            field_selector_dimension_3: FieldSelectorDimension3 { summary_field },
            field_selector_dimension_4: FieldSelectorDimension4 { trend_field },
        } => todo!(),
    }

    Ok(())
}
