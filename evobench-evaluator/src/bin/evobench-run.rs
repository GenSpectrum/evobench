use anyhow::{anyhow, bail, Result};
use clap::Parser;
use itertools::Itertools;

use std::path::PathBuf;

use evobench_evaluator::{
    config_file::{self, save_config_file, ConfigFile},
    get_terminal_width::get_terminal_width,
    key_val_fs::key_val::Entry,
    run::{
        benchmarking_job::BenchmarkingJobOpts,
        config::RunConfig,
        global_app_state_dir::GlobalAppStateDir,
        insert_jobs::{insert_jobs, open_already_inserted},
        run_job::{run_job, DryRun},
        run_queue::RunQueue,
        run_queues::{Never, RunQueues},
        working_directory_pool::WorkingDirectoryPool,
    },
    serde::{
        date_and_time::{system_time_to_rfc3339, DateTimeWithOffset},
        paths::ProperFilename,
    },
};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
/// Schedule (and query?) benchmarking jobs.
struct Opts {
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
    /// Show the supported config format types.
    ConfigFormats,

    /// Re-encode the config file (serialization type determined by
    /// file extension) and save at the given path.
    SaveConfig { output_path: PathBuf },

    /// Show the list of all inserted jobs, including already
    /// processed ones
    ListAll,

    /// List the currently scheduled and running jobs
    List,

    /// Insert a job
    Insert {
        #[clap(flatten)]
        benchmarking_job_opts: BenchmarkingJobOpts,

        /// Normally, the same job parameters can only be inserted
        /// once, subsequent attempts yield an error. This overrides
        /// the check and allows insertion anyway.
        #[clap(long)]
        force: bool,

        /// Exit quietly if the given job parameters were already
        /// inserted before (by default, give an error)
        #[clap(long)]
        quiet: bool,
    },

    /// Run the existing jobs
    Run {
        /// Show what is done
        #[clap(short, long)]
        verbose: bool,

        /// Do not run the jobs, but still consume the queue entries
        #[clap(short, long, default_value = "DoAll")]
        dry_run: DryRun,

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

/// Run through the queues forever, but pick up config changes
fn run_queues(
    config_path: Option<PathBuf>,
    mut conf: ConfigFile<RunConfig>,
    mut queues: RunQueues,
    mut working_directory_pool: WorkingDirectoryPool,
    verbose: bool,
    dry_run: DryRun,
    global_app_state_dir: &GlobalAppStateDir,
) -> Result<Never> {
    loop {
        // XX handle errors without exiting? Or do that above
        queues.run(verbose, |run_parameters| {
            run_job(&mut working_directory_pool, run_parameters, dry_run)
        })?;
        if conf.perhaps_reload_config(config_path.as_ref()) {
            // XXX only if changed
            eprintln!("reloaded configuration, re-initializing");
            // Drop locks before getting new ones
            drop(queues);
            drop(working_directory_pool);
            // XX handle errors without exiting? Or do that above
            queues = RunQueues::open(conf.queues.clone(), true, &global_app_state_dir)?;
            working_directory_pool = WorkingDirectoryPool::open(
                conf.working_directory_pool.clone(),
                conf.remote_repository.url.clone(),
                true,
                &|| global_app_state_dir.working_directory_pool_base(),
            )?;
        }
    }
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

    let conf = ConfigFile::<RunConfig>::load_config(config.as_ref(), |msg| {
        bail!("need a config file, {msg}")
    })?;

    let custom_parameters_set = conf
        .custom_parameters_set
        .checked(&conf.custom_parameters_required)?;

    let global_app_state_dir = GlobalAppStateDir::new()?;

    let queues = RunQueues::open(conf.queues.clone(), true, &global_app_state_dir)?;

    match subcommand {
        SubCommand::ConfigFormats => {
            println!(
                "These file extensions are supported: {}",
                config_file::supported_formats().join(", ")
            );
        }

        SubCommand::SaveConfig { output_path } => {
            save_config_file(&output_path, &*conf)?;
        }

        SubCommand::ListAll => {
            let already_inserted = open_already_inserted(&global_app_state_dir)?;

            let mut flat_jobs = Vec::new();
            for job in already_inserted
                .keys(false, None)?
                .map(|hash| -> Result<_> {
                    let hash = hash?;
                    Ok(already_inserted.get(&hash)?)
                })
                .filter_map(|r| r.transpose())
            {
                let (params, insertion_times) = job?;
                for t in insertion_times {
                    flat_jobs.push((params.clone(), t));
                }
            }
            flat_jobs.sort_by_key(|v| v.1);
            for (params, insertion_time) in flat_jobs {
                let t = system_time_to_rfc3339(insertion_time);
                println!("{t}\t{params:?}");
            }
        }

        SubCommand::List => {
            let show_queue = |i: &str, run_queue: &RunQueue| -> Result<()> {
                let RunQueue {
                    file_name,
                    schedule_condition,
                    queue,
                } = run_queue;

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
                Ok(())
            };

            // Originally COPY-PASTE from List action in jobqueue.rs, except
            // printing the job in :#? view on the next line.
            for (i, run_queue) in queues.pipeline().iter().enumerate() {
                show_queue(&i.to_string(), run_queue)?;
            }
            println!("------------------------------------------------------------------");
            if let Some(run_queue) = queues.erroneous_jobs_queue() {
                show_queue("erroneous_jobs_queue", run_queue)?;
            } else {
                println!(
                    "No erroneous_jobs_queue is configured (it would collect \
                     failing jobs"
                )
            }
            println!("------------------------------------------------------------------");
        }

        SubCommand::Insert {
            benchmarking_job_opts,
            force,
            quiet,
        } => {
            insert_jobs(
                benchmarking_job_opts.complete_jobs(&custom_parameters_set),
                &global_app_state_dir,
                &conf.remote_repository.url,
                force,
                quiet,
                &queues,
            )?;
        }

        SubCommand::Run {
            verbose,
            dry_run,
            mode,
        } => {
            let mut working_directory_pool = WorkingDirectoryPool::open(
                conf.working_directory_pool.clone(),
                conf.remote_repository.url.clone(),
                true,
                &|| global_app_state_dir.working_directory_pool_base(),
            )?;

            match mode {
                RunMode::Once {
                    stop_at,
                    queue_name,
                } => {
                    if let Some((run_queue, next_queue)) = queues.get_run_queue_by_name(&queue_name)
                    {
                        let stop_at = stop_at.map(|t| t.to_systemtime());
                        run_queue.run(
                            false,
                            verbose,
                            stop_at,
                            |run_parameters| {
                                run_job(&mut working_directory_pool, run_parameters, dry_run)
                            },
                            next_queue,
                            queues.erroneous_jobs_queue(),
                        )?;
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
                        run_queues(
                            config,
                            conf,
                            queues,
                            working_directory_pool,
                            verbose,
                            dry_run,
                            &global_app_state_dir,
                        )?;
                    }
                }
            }
        }
    }

    Ok(())
}
