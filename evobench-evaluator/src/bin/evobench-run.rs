use anyhow::{anyhow, bail, Result};
use clap::Parser;

use std::path::PathBuf;

use evobench_evaluator::{
    config_file::{save_config_file, LoadConfigFile},
    get_terminal_width::get_terminal_width,
    key::CheckedRunParameters,
    key_val_fs::key_val::Entry,
    lockable_file::StandaloneExclusiveFileLock,
    run::{
        benchmarking_job::BenchmarkingJobOpts,
        config::RunConfig,
        run_queue::RunQueue,
        run_queues::{Never, RunQueues},
    },
    serde::{date_and_time::DateTimeWithOffset, paths::ProperFilename},
};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
/// Schedule (and query?) benchmarking jobs.
struct Opts {
    /// Override the path to the config file (default: the paths
    /// `~/.evobench-run.json5` and `~/.evobench-run.json`, and if
    /// those are missing, use compiled-in default config values)
    #[clap(long)]
    config: Option<PathBuf>,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// Re-encode the config file (serialization type determined by
    /// file extension) and save at the given path.
    SaveConfig { output_path: PathBuf },

    /// List the current jobs
    List,
    /// Insert a job
    Insert {
        #[clap(flatten)]
        benchmarking_job_opts: BenchmarkingJobOpts,
    },
    /// Run the existing jobs
    Run {
        /// Show what is done
        #[clap(short, long)]
        verbose: bool,

        /// Do not run the jobs, but still consume the queue entries
        #[clap(short, long)]
        dry_run: bool,

        #[clap(subcommand)]
        mode: RunMode,
    },
}

#[derive(Debug, Clone, Copy, clap::Subcommand)]
pub enum DaemonizationAction {
    Start,
    /// XX by signal (forcefully) or some file (gracefully)?
    Stop,
    /// XX thankfully *can* just set the exit flag for one process,
    /// start a new one, will take over once the other is done, thanks
    /// to the locks
    Restart,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum RunMode {
    /// Run through the jobs in one queue, exit if there are no jobs
    /// left or, if given, the `stop_at` time is reached
    Once {
        /// Stop processing at the given time in the local time zone
        #[clap(short, long)]
        stop_at: Option<DateTimeWithOffset>,

        queue_name: ProperFilename,
    },
    /// Run forever, until terminated
    Daemon {
        /// Whether to background or stop the backgrounded daemon; if
        /// not given, runs in the foreground.
        #[clap(subcommand)]
        action: Option<DaemonizationAction>,
    },
}

fn main() -> Result<()> {
    let Opts { config, subcommand } = Opts::parse();

    // COPY-PASTE from List action in jobqueue.rs
    let get_filename = |entry: &Entry<_, _>| -> Result<String> {
        let file_name = entry.file_name();
        Ok(file_name
            .to_str()
            .ok_or_else(|| anyhow!("filename that cannot be decoded as UTF-8: {file_name:?}"))?
            .to_string())
    };

    let conf = RunConfig::load_config(config, |msg| bail!("need a config file, {msg}"))?;

    let queues = RunQueues::open(&conf.queues_config, true)?;

    match subcommand {
        SubCommand::SaveConfig { output_path } => {
            save_config_file(&output_path, &conf)?;
        }
        SubCommand::List => {
            // COPY-PASTE from List action in jobqueue.rs, except
            // printing the job in :#? view on the next line.
            for (
                i,
                RunQueue {
                    file_name,
                    schedule_condition,
                    queue,
                },
            ) in queues.run_queues().iter().enumerate()
            {
                println!("------------------------------------------------------------------");
                println!("{i}. Queue {file_name} ({schedule_condition:?}):");
                for entry in queue.sorted_entries(false, None) {
                    let mut entry = entry?;
                    let file_name = get_filename(&entry)?;
                    let key = entry.key()?;
                    let val = entry.get()?;
                    let locking = entry
                        .take_lockable_file()
                        .expect("not taken before")
                        .lock_status()?;
                    println!("\n{file_name} ({key})\t{locking}\n{val:#?}");
                }
            }
            println!("------------------------------------------------------------------");
        }
        SubCommand::Insert {
            benchmarking_job_opts,
        } => {
            let benchmarking_job =
                benchmarking_job_opts.checked(&conf.custom_parameters_required)?;
            queues.first().push_front(&benchmarking_job)?;
        }
        SubCommand::Run {
            verbose,
            dry_run,
            mode,
        } => {
            let base_dir = conf.queues_config.run_queues_basedir(false)?;
            let _lock = StandaloneExclusiveFileLock::try_lock_path(&base_dir, || {
                "another instance of evobench-run is already running".into()
            })?;

            let run_job = |checked_run_parameters| {
                if dry_run {
                    println!("dry-run: would run {checked_run_parameters:?}");
                } else {
                    let CheckedRunParameters {
                        commit_id,
                        checked_custom_parameters,
                    } = checked_run_parameters;

                    todo!()
                }
                Ok(())
            };

            let run_jobs =
                |queues: RunQueues<'_>| -> Result<Never> { queues.run(verbose, &run_job) };

            match mode {
                RunMode::Once {
                    stop_at,
                    queue_name,
                } => {
                    if let Some((run_queue, next_queue)) = queues.get_run_queue_by_name(&queue_name)
                    {
                        let stop_at = stop_at.map(|t| t.to_systemtime());
                        run_queue.run(false, verbose, stop_at, &run_job, next_queue)?;
                    } else {
                        bail!(
                            "unknown queue {queue_name} -- your configuration defines {:?}",
                            queues.queue_names()
                        )
                    }
                }
                RunMode::Daemon { action } => {
                    if let Some(action) = action {
                        todo!("daemonization {action:?}")
                    } else {
                        run_jobs(queues)?;
                        println!("this will never be executed"); // XXX remove.
                    }
                }
            }
        }
    }

    Ok(())
}
