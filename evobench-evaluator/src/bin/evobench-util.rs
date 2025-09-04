use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use evobench_evaluator::{
    get_terminal_width::get_terminal_width,
    git::GitHash,
    serde::proper_dirname::ProperDirname,
    util::grep_diff::GrepDiffRegion,
    utillib::logging::{set_log_level, LogLevelOpt},
};

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
        /// Filter for commit id
        #[clap(long, short)]
        commit: Option<GitHash>,

        /// Filter for target name
        #[clap(long, short)]
        target: Option<ProperDirname>,

        /// Filter for custom parameters (environment variables); you
        /// can provide multiple separated by '/',
        /// e.g. "FOO=1/BAR=hi"; not all of them need to be provided,
        /// the filter checks for existance and equality on those
        /// variables that are provided. NOTE: does not verify correct
        /// syntax of the variable names and values (currently no
        /// configuration is read, thus the info is not available)
        /// except for the basic acceptance for custom env var names.
        #[clap(long, short)]
        params: Option<String>,

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
            commit,
            target,
            params,
        } => {
            let grep_diff_region = GrepDiffRegion::from_strings(&regex_start, &regex_end)?;
            grep_diff_region.grep_diff(logfiles, commit, target, params)?;
        }
    }

    Ok(())
}
