use anyhow::{anyhow, bail, Result};
use clap::Parser;
use itertools::Itertools;
use run_git::git::GitWorkingDir;

use std::{
    borrow::Cow, fmt::Display, io::stdout, path::PathBuf, process::exit, str::FromStr, thread,
    time::Duration,
};

use evobench_evaluator::{
    config_file::{self, save_config_file, ConfigFile},
    get_terminal_width::get_terminal_width,
    git::GitHash,
    key::{RunParameters, RunParametersOpts},
    key_val_fs::key_val::Entry,
    lockable_file::LockStatus,
    run::{
        benchmarking_job::{
            BenchmarkingJobOpts, BenchmarkingJobReasonOpt, BenchmarkingJobSettingsOpts,
        },
        config::RunConfig,
        global_app_state_dir::GlobalAppStateDir,
        insert_jobs::{insert_jobs, open_already_inserted, ForceOpt, QuietOpt},
        polling_pool::PollingPool,
        run_context::RunContext,
        run_job::{run_job, DryRun},
        run_queue::RunQueue,
        run_queues::{get_now_chrono, RunQueues},
        working_directory_pool::WorkingDirectoryPool,
    },
    serde::{date_and_time::system_time_to_rfc3339, git_branch_name::GitBranchName},
    terminal_table::{TerminalTable, TerminalTableOpts, TerminalTableTitle},
    utillib::{
        logging::{set_log_level, LogLevelOpt},
        path_resolve_home::path_resolve_home,
    },
};

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width())]
/// Schedule (and query?) benchmarking jobs.
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

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
    ConfigSave { output_path: PathBuf },

    /// Show the list of all inserted jobs, including already
    /// processed ones
    ListAll {
        #[clap(flatten)]
        terminal_table_opts: TerminalTableOpts,
    },

    /// List the currently scheduled and running jobs
    List {
        #[clap(flatten)]
        terminal_table_opts: TerminalTableOpts,

        /// Show details, not just one item per line
        #[clap(short, long)]
        verbose: bool,

        /// Show all jobs in the extra queues (done and failures); by
        /// default, only the last `the lis` jobs are shown as stated
        /// in the QueuesConfig.
        #[clap(short, long)]
        all: bool,
    },

    /// Insert a job into the benchmarking queue. The given reference
    /// is resolved in a given working directory; if you have a commit
    /// id, then you can use the `insert` subcommand instead.
    InsertLocal {
        /// A Git reference to the commit that should be benchmarked
        /// (like `HEAD`, `master`, some commit id, etc.)
        reference: GitBranchName,

        /// The path to the Git working directory where `reference`
        /// should be resolved in
        #[clap(long, short, default_value = ".")]
        dir: PathBuf,

        #[clap(flatten)]
        benchmarking_job_settings: BenchmarkingJobSettingsOpts,
        #[clap(flatten)]
        reason: BenchmarkingJobReasonOpt,
        #[clap(flatten)]
        force_opt: ForceOpt,
        #[clap(flatten)]
        quiet_opt: QuietOpt,
    },

    /// Insert a job into the benchmarking queue, giving the commit id
    /// (hence, unlike the `insert-local` command, not requiring a
    /// working directory)
    Insert {
        #[clap(flatten)]
        benchmarking_job_opts: BenchmarkingJobOpts,

        #[clap(flatten)]
        force_opt: ForceOpt,
        #[clap(flatten)]
        quiet_opt: QuietOpt,
    },

    /// Insert jobs for new commits on configured branch names
    Poll {
        // No QuietOpt since that must be the default. Also, another
        // force option since the help text is different here.
        /// Normally, the same job parameters are only inserted once,
        /// subsequent polls yielding the same commits remain
        /// no-ops. This overrides the check and inserts the found
        /// commits anyway.
        #[clap(long)]
        force: bool,

        /// Suppress printing the "inserted n jobs" message when n >
        /// 0, i.e. always be quiet.
        #[clap(long)]
        quiet: bool,

        /// Do not report an error if any of the given (branch or
        /// other) names do not resolve.
        #[clap(long)]
        no_fail: bool,
    },

    /// Run the existing jobs
    Run {
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
    /// Run the single jobs that is first due.
    One {
        /// Exit with code 1 if there is no runnable job
        #[clap(long)]
        false_if_none: bool,
    },
    /// Run forever, until terminated
    Daemon {
        /// Whether to background or stop the backgrounded daemon; if
        /// not given, runs in the foreground.
        #[clap(subcommand)]
        action: Option<DaemonizationAction>,
    },
}

/// Run through the queues forever unless `once` is true (in which
/// case it returns true if a job was run), but pick up config changes
fn run_queues(
    config_path: Option<PathBuf>,
    mut conf: ConfigFile<RunConfig>,
    mut queues: RunQueues,
    mut working_directory_pool: WorkingDirectoryPool,
    dry_run: DryRun,
    global_app_state_dir: &GlobalAppStateDir,
    once: bool,
) -> Result<bool> {
    let mut run_context = RunContext::default();
    loop {
        // XX handle errors without exiting? Or do that above

        let ran = queues.run_next_job(
            |reason, run_parameters, queue| {
                run_job(
                    &mut working_directory_pool,
                    reason,
                    run_parameters,
                    &queue.schedule_condition,
                    dry_run,
                    &conf.benchmarking_command,
                    &path_resolve_home(&conf.output_base_dir)?,
                )
            },
            &mut run_context,
            get_now_chrono(),
        )?;

        if once {
            return Ok(ran);
        }

        thread::sleep(Duration::from_secs(5));

        if conf.perhaps_reload_config(config_path.as_ref()) {
            // XX only if changed
            eprintln!("reloaded configuration, re-initializing");
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
    let Opts {
        log_level,
        config,
        subcommand,
    } = Opts::parse();

    set_log_level(log_level.into());

    // COPY-PASTE from List action in jobqueue.rs
    let get_filename = |entry: &Entry<_, _>| -> Result<String> {
        let file_name = entry.file_name();
        Ok(file_name
            .to_str()
            .ok_or_else(|| anyhow!("filename that cannot be decoded as UTF-8: {file_name:?}"))?
            .to_string())
    };

    // Have to handle ConfigFormats before attempting to read the
    // config
    match &subcommand {
        SubCommand::ConfigFormats => {
            println!(
                "These configuration file extensions / formats are supported:\n\n  {}\n",
                config_file::supported_formats().join("\n  ")
            );
            return Ok(());
        }
        _ => (),
    }

    let conf = ConfigFile::<RunConfig>::load_config(config.as_ref(), |msg| {
        bail!("need a config file, {msg}")
    })?;

    let custom_parameters_set = conf
        .custom_parameters_set
        .checked(&conf.custom_parameters_required)?;

    let global_app_state_dir = GlobalAppStateDir::new()?;

    let queues = RunQueues::open(conf.queues.clone(), true, &global_app_state_dir)?;

    match subcommand {
        SubCommand::ConfigFormats => unreachable!("already dispatched above"),

        SubCommand::ConfigSave { output_path } => {
            save_config_file(&output_path, &*conf)?;
        }

        SubCommand::ListAll {
            terminal_table_opts,
        } => {
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
            let mut table = TerminalTable::start(
                &[38, 43],
                &[
                    TerminalTableTitle {
                        text: Cow::Borrowed("Insertion time"),
                        span: 1,
                    },
                    TerminalTableTitle {
                        text: Cow::Borrowed("Commit id"),
                        span: 1,
                    },
                    TerminalTableTitle {
                        text: Cow::Borrowed("Custom parameters"),
                        span: 1,
                    },
                ],
                terminal_table_opts,
                stdout().lock(),
            )?;
            for (params, insertion_time) in flat_jobs {
                let t = system_time_to_rfc3339(insertion_time);
                let RunParameters {
                    commit_id,
                    custom_parameters,
                } = params;
                let values: &[&dyn Display] = &[&t, &commit_id, &custom_parameters];
                table.write_data_row(values)?;
            }
            drop(table.finish()?);
        }

        SubCommand::List {
            verbose,
            terminal_table_opts,
            all,
        } => {
            let show_queue = |i: &str, run_queue: &RunQueue, is_extra_queue: bool| -> Result<()> {
                let RunQueue {
                    file_name,
                    schedule_condition,
                    queue,
                } = run_queue;

                // "Insertion time"
                // "locked" -- now just "R" or ""
                // priority
                // reason
                // "Commit id"
                // "Custom parameters"
                let titles = &[TerminalTableTitle {
                    text: format!("{i}: queue {file_name} ({schedule_condition}):").into(),
                    span: 6,
                }];
                let mut table = TerminalTable::start(
                    // t  R  pr rsn commit
                    &[37, 3, 4, 17, 42],
                    titles,
                    terminal_table_opts.clone(),
                    stdout().lock(),
                )?;

                let mut all_entries: Vec<_> = queue
                    .sorted_entries(false, None)
                    .collect::<Result<_, _>>()?;
                let entries: &mut [_] = if is_extra_queue && !all {
                    let max_len = conf.queues.view_jobs_max_len;
                    if all_entries.len() > max_len {
                        let skip = all_entries.len() - max_len;
                        if skip > 1 {
                            table.print(&format!("... ({skip} entries skipped)\n"))?;
                            &mut all_entries[skip..]
                        } else {
                            // skipping only 1 does not make sense thus show them all
                            &mut all_entries
                        }
                    } else {
                        &mut all_entries
                    }
                } else {
                    &mut all_entries
                };
                for entry in entries {
                    let file_name = get_filename(&entry)?;
                    let key = entry.key()?;
                    let job = entry.get()?;
                    let commit_id = &*job.run_parameters.commit_id.to_string();
                    let reason = if let Some(reason) = &job.reason {
                        reason.as_ref()
                    } else {
                        ""
                    };
                    let custom_parameters = &*job.run_parameters.custom_parameters.to_string();
                    let locking = if schedule_condition.is_grave_yard() {
                        ""
                    } else {
                        let lock_status = entry
                            .take_lockable_file()
                            .expect("not taken before")
                            .lock_status()?;
                        if lock_status == LockStatus::ExclusiveLock {
                            "R"
                        } else {
                            ""
                        }
                    };
                    let priority = &*job.priority.to_string();

                    if verbose {
                        table.write_data_row(&[
                            &*format!("{file_name} ({key})"),
                            locking,
                            priority,
                            reason,
                            commit_id,
                            custom_parameters,
                        ])?;
                        table.print(&format!("{job:#?}\n"))?;
                    } else {
                        table.write_data_row(&[
                            &*key.datetime().to_rfc3339(),
                            locking,
                            priority,
                            reason,
                            commit_id,
                            custom_parameters,
                        ])?;
                    }
                }
                drop(table.finish()?);
                Ok(())
            };

            let width = get_terminal_width();
            let bar_of = |c: u8| String::try_from([c].repeat(width)).expect("ascii char given");
            let thin_bar = bar_of(b'-');
            let thick_bar = bar_of(b'=');

            for (i, run_queue) in queues.pipeline().iter().enumerate() {
                println!("{thin_bar}");
                show_queue(&(i + 1).to_string(), run_queue, false)?;
            }
            println!("{thick_bar}");
            let perhaps_show_extra_queue =
                |queue_name: &str, queue_field: &str, run_queue: Option<&RunQueue>| -> Result<()> {
                    if let Some(run_queue) = run_queue {
                        show_queue(queue_name, run_queue, true)?;
                    } else {
                        println!("No {queue_field} is configured")
                    }
                    Ok(())
                };
            perhaps_show_extra_queue("done", "done_jobs_queue", queues.done_jobs_queue())?;
            println!("{thin_bar}");
            perhaps_show_extra_queue(
                "failures",
                "erroneous_jobs_queue",
                queues.erroneous_jobs_queue(),
            )?;
            println!("{thin_bar}");
        }

        SubCommand::InsertLocal {
            reason,
            reference,
            dir,
            benchmarking_job_settings,
            force_opt,
            quiet_opt,
        } => {
            let git_working_dir = GitWorkingDir::from(dir);
            let commit_id_str = git_working_dir
                .git_rev_parse(reference.as_str(), true)?
                .ok_or_else(|| anyhow!("reference '{reference}' does not resolve to a commit"))?;
            let commit_id = GitHash::from_str(&commit_id_str)?;

            let benchmarking_job_opts = BenchmarkingJobOpts {
                reason,
                benchmarking_job_settings,
                run_parameters: RunParametersOpts { commit_id },
            };

            insert_jobs(
                benchmarking_job_opts.complete_jobs(
                    Some(&conf.benchmarking_job_settings),
                    &custom_parameters_set,
                ),
                &global_app_state_dir,
                &conf.remote_repository.url,
                force_opt,
                quiet_opt,
                &queues,
            )?;
        }

        SubCommand::Insert {
            benchmarking_job_opts,
            force_opt,
            quiet_opt,
        } => {
            insert_jobs(
                benchmarking_job_opts.complete_jobs(
                    Some(&conf.benchmarking_job_settings),
                    &custom_parameters_set,
                ),
                &global_app_state_dir,
                &conf.remote_repository.url,
                force_opt,
                quiet_opt,
                &queues,
            )?;
        }

        SubCommand::Poll {
            force,
            quiet,
            no_fail,
        } => {
            let (commits, non_resolving) = {
                let mut polling_pool = PollingPool::open(
                    &conf.remote_repository.url,
                    &global_app_state_dir.working_directory_for_polling_pool_base()?,
                )?;

                let working_directory_id = polling_pool.updated_working_dir()?;
                polling_pool.resolve_branch_names(
                    working_directory_id,
                    &conf.remote_repository.remote_branch_names,
                )?
            };
            let num_commits = commits.len();

            let mut benchmarking_jobs = Vec::new();
            for (branch_name, commit_id) in commits {
                let opts = BenchmarkingJobOpts {
                    reason: BenchmarkingJobReasonOpt {
                        reason: branch_name.as_str().to_owned().into(),
                    },
                    benchmarking_job_settings: conf.benchmarking_job_settings.clone(),
                    run_parameters: RunParametersOpts { commit_id },
                };
                benchmarking_jobs.append(&mut opts.complete_jobs(
                    // already using conf.benchmarking_job_settings from above
                    None,
                    &custom_parameters_set,
                ));
            }

            let n_original = benchmarking_jobs.len();
            let n = insert_jobs(
                benchmarking_jobs,
                &global_app_state_dir,
                &conf.remote_repository.url,
                ForceOpt { force },
                // Must use quiet so that it can try to insert *all*
                // given jobs (XX: should it continue even with
                // errors, for the other code places?)
                QuietOpt { quiet: true },
                &queues,
            )?;

            if non_resolving.is_empty() || no_fail {
                if !quiet {
                    if n > 0 {
                        println!("inserted {n}/{n_original} jobs (for {num_commits} commits)");
                    }
                }
            } else {
                bail!(
                    "inserted {n}/{n_original} jobs (for {num_commits} commits), \
                     but the following names did not resolve: {non_resolving:?}"
                )
            }
        }

        SubCommand::Run { dry_run, mode } => {
            let working_directory_pool = WorkingDirectoryPool::open(
                conf.working_directory_pool.clone(),
                conf.remote_repository.url.clone(),
                true,
                &|| global_app_state_dir.working_directory_pool_base(),
            )?;

            match mode {
                RunMode::One { false_if_none } => {
                    let ran = run_queues(
                        config,
                        conf,
                        queues,
                        working_directory_pool,
                        dry_run,
                        &global_app_state_dir,
                        true,
                    )?;
                    if false_if_none {
                        let code = if ran { 0 } else { 1 };
                        exit(code)
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
                            dry_run,
                            &global_app_state_dir,
                            false,
                        )?;
                    }
                }
            }
        }
    }

    Ok(())
}
