use std::fmt::Debug;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_evaluator::evaluator::all_outputs_all_fields_table::AllOutputsAllFieldsTable;
use evobench_evaluator::evaluator::evaluator::{SingleRunStats, SummaryStats};
use evobench_evaluator::evaluator::options::{
    CheckedOutputOpts, EvaluationAndOutputOpts, FieldSelectorDimension3, FieldSelectorDimension4,
};
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
        evaluation_and_output_opts: EvaluationAndOutputOpts,

        /// The path that was provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        path: PathBuf,
    },

    /// Show statistics for a set of benchmarking log files, all for
    /// the same software version.
    Summary {
        #[clap(flatten)]
        evaluation_and_output_opts: EvaluationAndOutputOpts,
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
        evaluation_and_output_opts: EvaluationAndOutputOpts,
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
            evaluation_and_output_opts:
                EvaluationAndOutputOpts {
                    evaluation_opts,
                    output_opts,
                },
            path,
        } => {
            let CheckedOutputOpts {
                variants,
                flame_field,
            } = output_opts.check()?;
            let ldat = LogDataAndTree::read_file(&path, None)?;
            let aoaft = AllOutputsAllFieldsTable::from_log_data_tree(
                ldat.tree(),
                &evaluation_opts,
                variants,
                true,
            )?;
            aoaft.write_to_files(flame_field)?;
        }

        Command::Summary {
            evaluation_and_output_opts:
                EvaluationAndOutputOpts {
                    evaluation_opts,
                    output_opts,
                },
            paths,
            field_selector_dimension_3: FieldSelectorDimension3 { summary_field },
        } => {
            let CheckedOutputOpts {
                variants,
                flame_field,
            } = output_opts.check()?;
            let afts: Vec<AllOutputsAllFieldsTable<SingleRunStats>> = paths
                .par_iter()
                .map(|source_path| {
                    let ldat = LogDataAndTree::read_file(source_path, None)?;
                    AllOutputsAllFieldsTable::from_log_data_tree(
                        ldat.tree(),
                        &evaluation_opts,
                        variants.clone(),
                        false,
                    )
                })
                .collect::<Result<_>>()?;
            let aft = AllOutputsAllFieldsTable::<SummaryStats>::summary_stats(
                &afts,
                summary_field,
                &evaluation_opts,
                variants, // same as passed to from_log_data_tree above
                true,
            );
            aft.write_to_files(flame_field)?;
        }

        Command::Trend {
            evaluation_and_output_opts: evaluation_opts,
            grouped_paths,
            field_selector_dimension_3: FieldSelectorDimension3 { summary_field },
            field_selector_dimension_4: FieldSelectorDimension4 { trend_field },
        } => todo!(),
    }

    Ok(())
}
