use std::fmt::Debug;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use evobench_tools::evaluator::all_fields_table::{SingleRunStats, SummaryStats};
use evobench_tools::evaluator::all_outputs_all_fields_table::AllOutputsAllFieldsTable;
use evobench_tools::evaluator::data::log_data_and_tree::LogDataAndTree;
use evobench_tools::evaluator::options::{
    CheckedOutputOptions, EvaluationAndOutputOpts, FieldSelectorDimension3Opt,
    FieldSelectorDimension4Opt, FlameFieldOpt,
};
use evobench_tools::get_terminal_width::get_terminal_width;
use evobench_tools::stats::StatsField;
use evobench_tools::utillib::logging::{LogLevelOpt, set_log_level};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

include!("../../include/evobench_version.rs");

const PROGRAM_NAME: &str = "evobench-evaluator";

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

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
        field_selector_dimension_3: FieldSelectorDimension3Opt,
        #[clap(flatten)]
        flame_selector: FlameFieldOpt,

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
        field_selector_dimension_3: FieldSelectorDimension3Opt,
        #[clap(flatten)]
        field_selector_dimension_4: FieldSelectorDimension4Opt,
        #[clap(flatten)]
        flame_selector: FlameFieldOpt,

        /// The paths that were provided via the `EVOBENCH_LOG`
        /// environment variable to the evobench-probes library.
        grouped_paths: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let Opts { log_level, command } = Opts::parse();

    set_log_level(log_level.try_into()?);

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
            let CheckedOutputOptions { variants } = output_opts.check()?;
            let ldat = LogDataAndTree::read_file(&path)?;
            let aoaft = AllOutputsAllFieldsTable::from_log_data_tree(
                ldat.tree(),
                &evaluation_opts,
                variants,
                true,
            )?;
            aoaft.write_to_files(StatsField::Sum)?;
        }

        Command::Summary {
            evaluation_and_output_opts:
                EvaluationAndOutputOpts {
                    evaluation_opts,
                    output_opts,
                },
            paths,
            field_selector_dimension_3: FieldSelectorDimension3Opt { summary_field },
            flame_selector: FlameFieldOpt { flame_field },
        } => {
            let CheckedOutputOptions { variants } = output_opts.check()?;
            let afts: Vec<AllOutputsAllFieldsTable<SingleRunStats>> = paths
                .iter()
                .map(|source_path| {
                    let ldat = LogDataAndTree::read_file(source_path)?;
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

        #[allow(unused)]
        Command::Trend {
            evaluation_and_output_opts: evaluation_opts,
            grouped_paths,
            field_selector_dimension_3: FieldSelectorDimension3Opt { summary_field },
            field_selector_dimension_4: FieldSelectorDimension4Opt { trend_field },
            flame_selector: FlameFieldOpt { flame_field },
        } => todo!(),
    }

    Ok(())
}
