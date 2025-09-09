use std::{ffi::OsString, path::PathBuf};

use anyhow::{bail, Result};
use clap::Parser;

use evobench_evaluator::{
    ctx,
    get_terminal_width::get_terminal_width,
    git::GitHash,
    info,
    run::{
        config::{RunConfig, RunConfigWithReload},
        global_app_state_dir::GlobalAppStateDir,
        output_directory_structure::{KeyDir, RunDir},
        post_process::compress_file_as,
        working_directory_pool::WorkingDirectoryPoolBaseDir,
    },
    serde::proper_dirname::ProperDirname,
    util::grep_diff::GrepDiffRegion,
    utillib::logging::{set_log_level, LogLevelOpt},
};
use run_git::path_util::AppendToPath;

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Utilities for working with evobench
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

    // XX should wrap that help text (COPYPASTE) in a wrapper for flatten
    /// Override the path to the config file (default: the paths
    /// `~/.evobench-run.*` where a single one exists where the `*` is
    /// the suffix for one of the supported config file formats (run
    /// `config-formats` to get the list), and if those are missing,
    /// use compiled-in default config values)
    #[clap(long)]
    config: Option<PathBuf>,

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

    /// Do the same "single" post-processing on a single benchmark
    /// results as `evobench-run daemon` does--useful in case new
    /// features were added or the configuration was changed.
    PostProcessSingle {
        /// Skip (re)generation of the normal evobench.log Excel and
        /// flamegraph stats.
        #[clap(long)]
        no_stats: bool,

        /// The path to a directory for an individual run, i.e. ending
        /// in a directory name that is a timestamp
        run_dir: PathBuf,
    },

    /// Do the same "summary" post-processing on a set of benchmark
    /// results as `evobench-run daemon` does--useful in case new
    /// features were added or the configuration was changed.
    PostProcessSummary {
        /// Run `post-process-single` on all sub-directories for the
        /// individual runs for this 'key', too.
        #[clap(long)]
        single: bool,

        /// Skip (re)generation of the normal evobench.log Excel and
        /// flamegraph stats. (Only relevant when `--single` is
        /// given.)
        #[clap(long)]
        no_single_stats: bool,

        /// Skip (re)generation of the normal evobench.log Excel and
        /// flamegraph summary stats.
        #[clap(long)]
        no_summary_stats: bool,

        /// The path to a directory for a particular 'key', i.e. a set
        /// of individual runs: ending in a directory name that is a
        /// commit id
        key_dir: PathBuf,
    },
}

fn post_process_single(run_dir: &RunDir, run_config: &RunConfig, no_stats: bool) -> Result<()> {
    let target = run_dir.target_name()?;
    let standard_log_path = run_dir.standard_log_path();
    if !standard_log_path.exists() {
        info!(
            "missing {standard_log_path:?} -- try to find and move it \
             from the working directory pool dir"
        );

        let (_, _, _, date_time_with_offset) = run_dir.parse()?;
        let date_time_with_offset_str = date_time_with_offset.as_str();

        // (Is this too involved?)
        let global_app_state_dir = GlobalAppStateDir::new()?;
        let pool_base_dir =
            WorkingDirectoryPoolBaseDir::new(&run_config.working_directory_pool, &|| {
                global_app_state_dir.working_directory_pool_base()
            })?;
        let pool_base_dir_path = pool_base_dir.path();
        // /involved

        let found_log_file_name = {
            let mut file_names: Vec<OsString> = pool_base_dir_path
                .read_dir()
                .map_err(ctx!("reading dir {pool_base_dir_path:?}"))?
                .map(|entry| -> Result<_> {
                    let entry = entry?;
                    let file_name = entry.file_name();
                    if file_name
                        .to_string_lossy()
                        .contains(date_time_with_offset_str)
                    {
                        Ok(Some(file_name))
                    } else {
                        Ok(None)
                    }
                })
                .filter_map(|v| v.transpose())
                .collect::<Result<_>>()?;
            match file_names.len() {
                1 => file_names.pop().expect("seen"),
                0 => bail!(
                    "can't find standard log at {standard_log_path:?} and finding \
                     {date_time_with_offset_str:?} in {pool_base_dir_path:?} was unsuccessful"
                ),
                _ => bail!(
                    "got more than one match for {date_time_with_offset_str:?} in \
                     {pool_base_dir_path:?}"
                ),
            }
        };

        let found_log_file_path = pool_base_dir_path.append(&found_log_file_name);
        info!("found file {found_log_file_path:?}");

        compress_file_as(&found_log_file_path, standard_log_path.clone(), false)?;
        std::fs::remove_file(&found_log_file_path)?;
        info!("deleted moved file {found_log_file_path:?}");
    }
    run_dir.post_process_single(
        None,
        || Ok(()),
        &target,
        &standard_log_path,
        run_config,
        no_stats,
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let Opts {
        config,
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

        SubCommand::PostProcessSingle { run_dir, no_stats } => {
            let run_config_with_reload = RunConfigWithReload::load(config.as_ref(), |msg| {
                bail!("can't load config: {msg}")
            })?;
            let run_config = &run_config_with_reload.run_config;

            let run_dir = RunDir::try_from(run_dir)?;

            post_process_single(&run_dir, run_config, no_stats)?;
        }

        SubCommand::PostProcessSummary {
            single,
            key_dir,
            no_single_stats,
            no_summary_stats,
        } => {
            let run_config_with_reload = RunConfigWithReload::load(config.as_ref(), |msg| {
                bail!("can't load config: {msg}")
            })?;
            let run_config = &run_config_with_reload.run_config;

            let key_dir = KeyDir::try_from(key_dir)?;

            if single {
                for run_dir in key_dir.run_dirs()? {
                    post_process_single(&run_dir, run_config, no_single_stats)?;
                }
            }

            key_dir.generate_summaries_for_key_dir(no_summary_stats)?;
        }
    }

    Ok(())
}
