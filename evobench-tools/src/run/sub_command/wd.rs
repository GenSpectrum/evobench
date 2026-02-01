use std::{borrow::Cow, env, ffi::OsStr, io::stdout, process::exit, sync::Arc, time::SystemTime};

use anyhow::{Result, anyhow, bail};
use chj_unix_util::polling_signals::PollingSignals;
use cj_path_util::path_util::AppendToPath;
use itertools::Itertools;

use crate::{
    ask::ask_yn,
    ctx,
    io_utils::bash::{bash_export_variable_string, bash_string_from_program_path_and_args},
    key::{BenchmarkingJobParameters, RunParameters},
    lazyresult,
    lockable_file::{StandaloneExclusiveFileLock, StandaloneFileLockError},
    run::{
        command_log_file::CommandLogFile,
        config::{BenchmarkingCommand, RunConfig},
        dataset_dir_env_var::dataset_dir_for,
        env_vars::assert_evobench_env_var,
        run_job::get_commit_tags,
        sub_command::{open_working_directory_pool, wd_log::LogOrLogf},
        versioned_dataset_dir::VersionedDatasetDir,
        working_directory::{FetchedTags, Status, WorkingDirectory, WorkingDirectoryStatus},
        working_directory_pool::{
            WdAllowBareOpt, WorkingDirectoryId, WorkingDirectoryIdOpt, WorkingDirectoryPoolBaseDir,
            finish_parsing_working_directory_ids,
        },
    },
    serde::date_and_time::system_time_to_rfc3339,
    terminal_table::{TerminalTable, TerminalTableOpts, TerminalTableTitle},
    utillib::unix::ToExitCode,
    warn,
};

pub fn open_working_directory_change_signals(conf: &RunConfig) -> Result<PollingSignals> {
    let signals_path = conf.working_directory_change_signals_path();
    PollingSignals::open(&signals_path, 0).map_err(ctx!("opening signals path {signals_path:?}"))
}

#[derive(Debug, thiserror::Error)]
pub enum GetRunLockError {
    #[error("{0}")]
    AlreadyLocked(StandaloneFileLockError),
    #[error("{0}")]
    Generic(anyhow::Error),
}

// omg the error handling.
pub fn get_run_lock(conf: &RunConfig) -> Result<StandaloneExclusiveFileLock, GetRunLockError> {
    let run_lock_path = &conf.run_jobs_daemon.state_dir;

    match StandaloneExclusiveFileLock::try_lock_path(run_lock_path, || {
        "getting the global lock for running jobs".into()
    }) {
        Ok(run_lock) => Ok(run_lock),
        Err(e) => match &e {
            StandaloneFileLockError::IOError { path: _, error: _ } => {
                Err(GetRunLockError::Generic(e.into()))
            }
            StandaloneFileLockError::AlreadyLocked { path: _, msg: _ } => {
                Err(GetRunLockError::AlreadyLocked(e))
            }
        },
    }
}

/// Checks via temporary flock. XX should use shared for this, oh
/// my. Already have such code in flock module, too! Make properly
/// usable.
fn daemon_is_running(conf: &RunConfig) -> Result<bool> {
    match get_run_lock(conf) {
        Ok(_) => Ok(false),
        Err(e) => match &e {
            GetRunLockError::AlreadyLocked(_) => Ok(true),
            GetRunLockError::Generic(_) => Err(e.into()),
        },
    }
}

#[derive(Debug, clap::Subcommand)]
pub enum Wd {
    /// List the working directories; by default, show all of them
    List {
        #[clap(flatten)]
        terminal_table_opts: TerminalTableOpts,

        /// Show the active working directories
        #[clap(long)]
        active: bool,

        /// Show the working directories that have been set aside due to errors
        #[clap(long)]
        error: bool,

        /// Sort the list by the id (numerically). Default: sort by the `last_used` timestamp
        #[clap(short, long)]
        numeric_sort: bool,

        /// Only show the ID of the working directories
        #[clap(short, long)]
        id_only: bool,

        /// Do not show the column with the checked-out commid id
        /// (speeds up the listing)
        #[clap(long)]
        no_commit: bool,
    },
    /// Delete working directories that have been set aside due to
    /// errors
    Cleanup {
        /// Do not actually delete, just show the directories
        #[clap(long)]
        dry_run: bool,

        /// Show the list of ids of working directories that were
        /// deleted
        #[clap(short, long)]
        verbose: bool,

        /// Which of the working directories with errors to delete
        #[clap(subcommand)]
        mode: WdCleanupMode,
    },
    /// *Immediately* delete working directories
    Delete {
        /// Do not actually delete, just show the directory paths
        #[clap(long)]
        dry_run: bool,

        /// Delete directories even if they are not in "error" status
        #[clap(short, long)]
        force: bool,

        /// Show the list of ids of working directories that were
        /// deleted
        #[clap(short, long)]
        verbose: bool,

        #[clap(flatten)]
        allow_bare: WdAllowBareOpt,

        /// Which of the working directories in error status to
        /// immediately delete. Refuses directories with different
        /// status than `error` unless `--force` was given.
        ids: Vec<WorkingDirectoryIdOpt>,
    },
    /// Open the log file for the last run in a working directory in
    /// the `PAGER` (or `less`)
    Log(LogOrLogf),
    /// Open the log file for the last run in a working directory in
    /// `tail -f`.
    Logf(LogOrLogf),
    /// Mark the given working directories for examination, so that
    /// they are not deleted by `evobench wd cleanup`
    Mark {
        #[clap(flatten)]
        allow_bare: WdAllowBareOpt,

        /// The IDs of the working direcories to mark
        ids: Vec<WorkingDirectoryIdOpt>,
    },
    /// Change the status of the given working directories back to
    /// "error", so that they are again deleted by `evobench wd
    /// cleanup`
    Unmark {
        #[clap(flatten)]
        allow_bare: WdAllowBareOpt,

        /// The IDs of the working direcories to unmark
        ids: Vec<WorkingDirectoryIdOpt>,
    },
    /// Change the status of the given working directories back to
    /// "checkedout", so that they can be used again by `evobench
    /// run`. (Be careful that you don't recycle dirs with problems
    /// that lead to errors again. It may be safer, albeit costlier,
    /// to `delete` the dirs instead.)
    Recycle {
        #[clap(flatten)]
        allow_bare: WdAllowBareOpt,

        /// The IDs of the working direcories to recycle
        ids: Vec<WorkingDirectoryIdOpt>,
    },
    /// Mark the given working directory for examination, then open a
    /// shell inside it. The shell in the `SHELL` environment variable
    /// is used, falling back to "bash".
    Enter {
        /// Keep the working directory marked for examination even
        /// after exiting the shell (default: ask interactively)
        #[clap(long)]
        mark: bool,

        /// Unmark the working directory after exiting the shell
        /// (without asking, and even if the directory was marked)
        #[clap(long)]
        unmark: bool,

        /// Force entering even when the status is `processing`; in this case,
        #[clap(long)]
        force: bool,

        /// Do not run `git fetch --tags` inside the working directory
        /// (usually it's a good idea to run it, to ensure the dataset
        /// dir and `COMMIT_TAGS` are chosen based on up to date
        /// remote data)
        #[clap(long)]
        no_fetch: bool,

        #[clap(flatten)]
        allow_bare: WdAllowBareOpt,

        /// The ID of the working directory to mark and enter
        id: WorkingDirectoryIdOpt,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum WdCleanupMode {
    /// Delete all working directories with errors
    All,
    /// Delete those that were set aside at least the given number of
    /// days ago
    StaleForDays {
        /// Number of days (can be a floating point value)
        x: f32,
    },
}

impl Wd {
    pub fn run(
        self,
        conf: &RunConfig,
        working_directory_base_dir: &Arc<WorkingDirectoryPoolBaseDir>,
    ) -> Result<()> {
        let mut working_directory_pool =
            open_working_directory_pool(conf, working_directory_base_dir.clone())?
                // XX might we want to hold onto the lock?
                .into_inner();

        let check_original_status =
            |wd: &WorkingDirectory, allowed_statuses: &str| -> Result<Status> {
                let status = wd.working_directory_status.status;
                if status.can_be_used_for_jobs() {
                    bail!(
                        "this action is only for working directories in {allowed_statuses} \
                         status, but directory {} has status '{}'",
                        wd.working_directory_path().parent_path_and_id()?.1,
                        status
                    )
                    // Also can't currently signal working dir status
                    // changes to the running daemon, only Error and
                    // Examination are safe as those are ignored by
                    // the daemon
                } else {
                    Ok(status)
                }
            };

        #[derive(Debug, thiserror::Error)]
        enum DoMarkError {
            #[error("{0}")]
            Check(anyhow::Error),
            #[error("{0}")]
            Generic(anyhow::Error),
        }

        enum Marked {
            OldStatus(Status),
            Unchanged,
        }

        // When giving a status that can be used by the daemon
        // (Recycle action), working_directory_change_signals has
        // to be passed in. Returns None if the working directory
        // does not exist.
        let mut do_mark = |wanted_status: Status,
                           ignore_if_already_wanted_status: bool,
                           id: WorkingDirectoryId,
                           working_directory_change_signals: Option<&mut PollingSignals>|
         -> Result<Option<Marked>, DoMarkError> {
            let mut guard = working_directory_pool
                .lock_mut("evobench SubCommand::Wd do_mark")
                .map_err(DoMarkError::Generic)?;
            if let Some(mut wd) = guard.get_working_directory_mut(id) {
                if ignore_if_already_wanted_status
                    && wd.working_directory_status.status == wanted_status
                {
                    return Ok(Some(Marked::Unchanged));
                }
                let original_status = check_original_status(&*wd, "error/examination")
                    .map_err(ctx!("refusing working directory {id}"))
                    .map_err(DoMarkError::Check)?;
                wd.set_and_save_status(wanted_status)
                    .map_err(DoMarkError::Generic)?;
                if let Some(working_directory_change_signals) = working_directory_change_signals {
                    working_directory_change_signals.send_signal();
                }
                Ok(Some(Marked::OldStatus(original_status)))
            } else {
                Ok(None)
            }
        };

        let mut working_directory_change_signals =
            lazyresult!(open_working_directory_change_signals(conf));

        match self {
            Wd::List {
                terminal_table_opts,
                active,
                error,
                id_only,
                no_commit,
                numeric_sort,
            } => {
                let widths = &[3 + 2, Status::MAX_STR_LEN + 2, 8 + 2, 35 + 2, 35 + 2];
                let titles = &[
                    "id",
                    "status",
                    "num_runs",
                    "creation_timestamp",
                    "last_use",
                    "commit_id",
                ]
                .map(|s| TerminalTableTitle {
                    text: Cow::Borrowed(s),
                    span: 1,
                });
                fn used<T>(vals: &[T], show_commit: bool) -> &[T] {
                    if show_commit {
                        vals
                    } else {
                        &vals[..vals.len() - 1]
                    }
                }

                let mut table = if id_only {
                    None
                } else {
                    Some(TerminalTable::start(
                        used(widths, !no_commit),
                        used(titles, !no_commit),
                        None,
                        terminal_table_opts,
                        stdout().lock(),
                    )?)
                };

                let all_ids: Vec<_> = {
                    let mut all_entries: Vec<_> = working_directory_pool.all_entries().collect();
                    if numeric_sort {
                        // Leave as is, it's already sorted
                    } else {
                        all_entries.sort_by(|a, b| a.1.last_use.cmp(&b.1.last_use))
                    }
                    all_entries.iter().map(|(id, _)| *id).collect()
                };

                for id in all_ids {
                    let mut lock = working_directory_pool.lock_mut("evobench Wd::List")?;
                    let mut wd = lock
                        .get_working_directory_mut(id)
                        .expect("got it from all_entries");
                    let WorkingDirectoryStatus {
                        creation_timestamp,
                        num_runs,
                        status,
                    } = &wd.working_directory_status;

                    let show = match (active, error) {
                        (true, true) | (false, false) => true,
                        (true, false) => status.can_be_used_for_jobs(),
                        (false, true) => !status.can_be_used_for_jobs(),
                    };
                    if show {
                        if let Some(table) = &mut table {
                            let row = &[
                                id.to_string(),
                                status.to_string(),
                                num_runs.to_string(),
                                creation_timestamp.to_string(),
                                system_time_to_rfc3339(wd.last_use, true),
                                if !no_commit {
                                    wd.commit()?.to_string()
                                } else {
                                    String::new()
                                },
                            ];
                            table.write_data_row(used(row, !no_commit), None)?;
                        } else {
                            println!("{id}");
                        }
                    }
                }

                if let Some(table) = table {
                    let _ = table.finish()?;
                }
                Ok(())
            }
            Wd::Cleanup {
                dry_run,
                verbose,
                mode,
            } => {
                let stale_days = match mode {
                    WdCleanupMode::All => 0.,
                    WdCleanupMode::StaleForDays { x } => x,
                };
                if stale_days < 0. {
                    bail!("number of days must be non-negative");
                }
                if stale_days > 1000. || stale_days.is_nan() {
                    bail!("number of days must be reasonable");
                }

                let stale_seconds = (stale_days * 24. * 3600.) as u64;

                let now = SystemTime::now();

                let mut cleanup_ids = Vec::new();
                for (id, wd) in working_directory_pool
                    .all_entries()
                    .filter(|(_, wd)| wd.working_directory_status.status == Status::Error)
                {
                    let d = now.duration_since(wd.last_use).map_err(ctx!(
                        "calculating time since last use of working directory {id}"
                    ))?;
                    if d.as_secs() > stale_seconds {
                        cleanup_ids.push(id);
                    }
                }

                {
                    let mut lock = working_directory_pool.lock_mut("evobench Wd::Cleanup")?;
                    for id in cleanup_ids {
                        if dry_run {
                            eprintln!("would delete working directory {id}");
                        } else {
                            // XX Note: can this fail if a concurrent
                            // instance deletes it in the mean time?
                            lock.delete_working_directory(id)?;
                        }
                        if verbose {
                            println!("{id}");
                        }
                    }
                }
                Ok(())
            }
            Wd::Delete {
                dry_run,
                force,
                verbose,
                ids,
                allow_bare,
            } => {
                let ids = finish_parsing_working_directory_ids(ids, allow_bare)?;

                let mut lock_mut = working_directory_pool.lock_mut("evobench Wd::Delete")?;
                let opt_current_wd_id = lock_mut
                    .locked_base_dir()
                    .read_current_working_directory()?;
                for id in ids {
                    let lock = lock_mut.shared();
                    let wd = lock
                        .get_working_directory(id)
                        .ok_or_else(|| anyhow!("working directory {id} does not exist"))?;
                    let status = wd.working_directory_status.status;
                    if force {
                        if Some(id) == opt_current_wd_id {
                            // XX add abstraction for this
                            let status_is_in_use = match status {
                                Status::CheckedOut => true, // XX might change
                                Status::Processing => true,
                                Status::Error => false,
                                Status::Finished => false,
                                Status::Examination => false,
                            };
                            if status_is_in_use {
                                if daemon_is_running(conf)? {
                                    bail!(
                                        "working directory {id} is in use and \
                                             the daemon is running"
                                    );
                                } else {
                                    // Allow
                                }
                            }
                        }
                    } else {
                        if status != Status::Error {
                            let tip = if status == Status::Examination {
                                "; please first use the `unmark` action to move it \
                                     out of examination"
                            } else {
                                "; use the `--force` option if you're sure"
                            };
                            bail!(
                                "working directory {id} is not in `error`, but `{status}` \
                                     status{tip}"
                            );
                        }
                    }
                    if dry_run {
                        let path = wd.git_working_dir.working_dir_path_ref();
                        eprintln!("would delete working directory at {path:?}");
                    } else {
                        if status.can_be_used_for_jobs() {
                            working_directory_change_signals.force_mut()?.send_signal();
                            // No race possible since we're
                            // holding the working dir pool lock,
                            // right?
                        }

                        // Note: can this fail if a concurrent
                        // instance deletes it in the mean time?
                        // But can't since each instance must be
                        // holding the pool lock, right?
                        lock_mut.delete_working_directory(id)?;
                        if verbose {
                            println!("{id}");
                        }
                    }
                }
                Ok(())
            }
            Wd::Log(opts) => {
                opts.run(false, &working_directory_pool)?;
                Ok(())
            }
            Wd::Logf(opts) => {
                opts.run(true, &working_directory_pool)?;
                Ok(())
            }
            Wd::Mark { ids, allow_bare } => {
                let ids = finish_parsing_working_directory_ids(ids, allow_bare)?;
                for id in ids {
                    if do_mark(Status::Examination, true, id, None)?.is_none() {
                        warn!("there is no working directory for id {id}");
                    }
                }
                Ok(())
            }
            Wd::Unmark { ids, allow_bare } => {
                let ids = finish_parsing_working_directory_ids(ids, allow_bare)?;
                for id in ids {
                    if do_mark(Status::Error, true, id, None)?.is_none() {
                        warn!("there is no working directory for id {id}");
                    }
                }
                Ok(())
            }
            Wd::Recycle { ids, allow_bare } => {
                let ids = finish_parsing_working_directory_ids(ids, allow_bare)?;
                for id in ids {
                    if do_mark(
                        Status::CheckedOut,
                        true,
                        id,
                        Some(working_directory_change_signals.force_mut()?),
                    )?
                    .is_none()
                    {
                        warn!("there is no working directory for id {id}");
                    }
                }
                Ok(())
            }
            Wd::Enter {
                mark,
                unmark,
                force,
                no_fetch,
                id,
                allow_bare,
            } => {
                let id = id.to_working_directory_id(allow_bare)?;
                if mark && unmark {
                    bail!("please only give one of the --mark or --unmark options")
                }

                let no_exist = || anyhow!("there is no working directory for id {id}");

                // Try to change the status; if it's in an
                // unacceptable status, enter anyway if `force` is
                // given, but don't restore it then
                let original_status: Option<Status> =
                    match do_mark(Status::Examination, false, id, None) {
                        Ok(status) => {
                            if let Some(status) = status {
                                match status {
                                    Marked::OldStatus(status) => Some(status),
                                    Marked::Unchanged => unreachable!("we gave it false"),
                                }
                            } else {
                                Err(no_exist())?
                            }
                        }
                        Err(DoMarkError::Check(e)) => {
                            if force {
                                None
                            } else {
                                Err(e)?
                            }
                        }
                        Err(DoMarkError::Generic(e)) => Err(e)?,
                    };

                let working_directory = working_directory_pool
                    .get_working_directory(id)
                    .ok_or_else(&no_exist)?;

                let (standard_log_path, _id) = working_directory
                    .working_directory_path()
                    .last_standard_log_path()?
                    .ok_or_else(|| {
                        anyhow!("could not find a log file for working directory {id}")
                    })?;

                let command_log_file = CommandLogFile::from(&standard_log_path);
                let command_log = command_log_file.command_log()?;

                let BenchmarkingJobParameters {
                    run_parameters,
                    command,
                } = command_log.parse_log_file_params()?;

                let RunParameters {
                    commit_id,
                    custom_parameters,
                } = &*run_parameters;

                let BenchmarkingCommand {
                    target_name: _,
                    subdir,
                    command,
                    arguments,
                    pre_exec_bash_code,
                } = &*command;

                let fetched_tags = if no_fetch {
                    // Just pretend that they were fetched (they
                    // were, just further in the past, OK?)
                    FetchedTags::Yes
                } else {
                    working_directory.fetch(Some(commit_id))?
                };

                let commit_tags = get_commit_tags(
                    working_directory,
                    commit_id,
                    &conf.commit_tags_regex,
                    fetched_tags.clone(),
                )?;

                let mut vars: Vec<(&str, &OsStr)> = custom_parameters
                    .btree_map()
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_ref()))
                    .collect();

                let check = assert_evobench_env_var;

                let commit_id_str = commit_id.to_string();
                vars.push((check("COMMIT_ID"), commit_id_str.as_ref()));
                vars.push((check("COMMIT_TAGS"), commit_tags.as_ref()));

                let versioned_dataset_dir = VersionedDatasetDir::new();
                let dataset_dir_;
                if let Some(dataset_dir) = dataset_dir_for(
                    conf.versioned_datasets_base_dir.as_deref(),
                    &custom_parameters,
                    &versioned_dataset_dir,
                    &working_directory.git_working_dir,
                    &commit_id,
                    fetched_tags,
                )? {
                    dataset_dir_ = dataset_dir;
                    vars.push((check("DATASET_DIR"), dataset_dir_.as_ref()));
                }

                let exports = vars
                    .iter()
                    .map(|(k, v)| bash_export_variable_string(k, &v.to_string_lossy(), "  ", "\n"))
                    .join("");

                let shell = match std::env::var("SHELL") {
                    Ok(s) => s,
                    Err(e) => match e {
                        env::VarError::NotPresent => "bash".into(),
                        env::VarError::NotUnicode(os_string) => {
                            bail!("the SHELL environment variable is not in unicode: {os_string:?}")
                        }
                    },
                };

                // -- Print explanations ----

                println!(
                    "The log file from this job execution is:\n\
                         {standard_log_path:?}\n"
                );

                if shell != "bash" && shell != "/bin/bash" {
                    println!(
                        "Note: SHELL is set to {shell:?}, but the following syntax \
                             is for bash.\n"
                    );
                }

                println!("The following environment variables have been set:\n\n{exports}");

                println!(
                    "To rerun the benchmarking, please set `BENCH_OUTPUT_LOG` \
                         and optionally `EVOBENCH_LOG` to some suitable paths, \
                         then run:\n\n  {}\n",
                    bash_string_from_program_path_and_args(command, arguments)
                );

                let actual_commit = working_directory.git_working_dir.get_head_commit_id()?;
                if commit_id_str != actual_commit {
                    println!(
                        "*** WARNING: the checked-out commit in this directory \
                             does not match the commit id for the job! ***\n"
                    );
                }

                if original_status.is_none() {
                    println!(
                        "*** WARNING: processing is ongoing, entering this directory \
                             by force! Please do not hinder the benchmarking process! ***\n"
                    );
                }

                // Enter dir without any locking (other than dir
                // being in Status::Examination now), OK?

                let mut cmd = pre_exec_bash_code
                    .to_run_with_pre_exec(conf)
                    .command::<&str>(&shell, []);
                cmd.envs(vars);
                cmd.current_dir(
                    working_directory
                        .git_working_dir
                        .working_dir_path_ref()
                        .append(subdir),
                );
                let status = cmd.status()?;

                if unmark || original_status != Some(Status::Examination) {
                    if mark {
                        // keep marked
                    } else {
                        if let Some(original_status) = original_status {
                            let do_revert = unmark
                                || ask_yn(&format!(
                                    "Should the working directory status be reverted to \
                                         '{original_status}' (i.e. are you done)?"
                                ))?;

                            if do_revert {
                                let mut wd = working_directory_pool
                                    .lock_mut("evobench Wd::Enter do_revert")?
                                    .into_get_working_directory_mut(id);
                                let mut working_directory = wd.get().ok_or_else(|| {
                                    anyhow!("there is no working directory for id {id}")
                                })?;
                                let wanted_status = Status::Error;
                                assert!(
                                    original_status == wanted_status
                                        || original_status == Status::Examination
                                );
                                working_directory.set_and_save_status(wanted_status)?;
                                println!("Changed status to '{wanted_status}'");
                            } else {
                                println!("Leaving status at 'examination'");
                            }
                        } else {
                            // original status was unacceptable
                            // and `force` was given, thus do not
                            // restore
                        }
                    }
                } else {
                    if !mark {
                        let status_str = if let Some(original_status) = original_status {
                            &original_status.to_string()
                        } else {
                            "processing(?)"
                        };
                        println!(
                            "Leaving working directory status at the original status, \
                                 {status_str}",
                        );
                    }
                }

                exit(status.to_exit_code());
            }
        }
    }
}
