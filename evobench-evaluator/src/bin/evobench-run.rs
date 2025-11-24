use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Local};
use clap::Parser;
use itertools::Itertools;
use lazy_static::lazy_static;
use run_git::{git::GitWorkingDir, path_util::AppendToPath};
use yansi::{Color, Style};

use std::{
    borrow::Cow,
    env,
    ffi::OsStr,
    fmt::Display,
    io::{stdout, IsTerminal, Write},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::PathBuf,
    process::{exit, Command},
    str::FromStr,
    thread,
    time::{Duration, SystemTime},
};

use evobench_evaluator::{
    ask::ask_yn,
    config_file::{self, ron_to_string_pretty, save_config_file},
    ctx,
    date_and_time::system_time_with_display::SystemTimeWithDisplay,
    debug,
    get_terminal_width::get_terminal_width,
    git::GitHash,
    info,
    io_utils::bash::{bash_export_variable_string, bash_string_from_program_path_and_args},
    key::{BenchmarkingJobParameters, RunParameters, RunParametersOpts},
    key_val_fs::key_val::Entry,
    lockable_file::{LockStatus, StandaloneExclusiveFileLock},
    run::{
        benchmarking_job::{
            BenchmarkingJobOpts, BenchmarkingJobReasonOpt, BenchmarkingJobSettingsOpts,
        },
        command_log_file::CommandLogFile,
        config::{BenchmarkingCommand, RunConfigWithReload},
        dataset_dir_env_var::dataset_dir_for,
        global_app_state_dir::GlobalAppStateDir,
        insert_jobs::{insert_jobs, open_already_inserted, ForceOpt, QuietOpt},
        polling_pool::PollingPool,
        run_context::RunContext,
        run_job::JobRunner,
        run_queue::RunQueue,
        run_queues::RunQueues,
        versioned_dataset_dir::VersionedDatasetDir,
        working_directory::{Status, WorkingDirectory, WorkingDirectoryStatus},
        working_directory_pool::{
            WorkingDirectoryId, WorkingDirectoryPool, WorkingDirectoryPoolBaseDir,
        },
    },
    serde::{
        date_and_time::{system_time_to_rfc3339, DateTimeWithOffset},
        git_branch_name::GitBranchName,
    },
    terminal_table::{TerminalTable, TerminalTableOpts, TerminalTableTitle},
    utillib::{
        arc::CloneArc,
        logging::{set_log_level, LogLevelOpt},
        re_exec::re_exec_with_existing_args_and_env,
        unix::ToExitCode,
    },
    warn,
};

fn unicode_is_fine() -> bool {
    (|| -> Option<bool> {
        let term = env::var_os("TERM")?;
        let lang = env::var_os("LANG")?;
        let lang = lang.to_str()?;
        Some(term.as_bytes() == b"xterm" && lang.contains("UTF-8"))
    })()
    .unwrap_or(false)
}

lazy_static! {
    static ref UNICODE_IS_FINE: bool = unicode_is_fine();
}

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Schedule and query benchmarking jobs.
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
        /// default, only the last `view_jobs_max_len` jobs are shown
        /// as stated in the QueuesConfig.
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

    /// Insert jobs for new commits on branch names configured in the
    /// config option `remote_branch_names_for_poll`
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

    /// Run the existing jobs; this takes a lock or stops with an
    /// error if the lock is already taken
    Run {
        #[clap(subcommand)]
        mode: RunMode,
    },

    /// Handle working directories
    Wd {
        /// The subcommand to run. Use `--help` after the sub-command to
        /// get a list of the allowed options there.
        #[clap(subcommand)]
        subcommand: WdSubCommand,
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

        /// Check if the evobench-run binary is changed (older or
        /// newer modification time), and if so, re-execute it with
        /// the original arguments.
        #[clap(long)]
        restart_on_upgrades: bool,
    },
}

#[derive(Debug, clap::Subcommand)]
enum WdSubCommand {
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

        /// Sort the list by the `last_used` timestamp
        #[clap(short, long)]
        sort_used: bool,

        /// Only show the ID of the working directories
        #[clap(short, long)]
        id_only: bool,
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
        mode: WdSubCommandCleanupMode,
    },
    /// *Immediately* delete working directories
    Delete {
        /// Do not actually delete, just show the directory paths
        #[clap(long)]
        dry_run: bool,

        /// Show the list of ids of working directories that were
        /// deleted
        #[clap(short, long)]
        verbose: bool,

        /// Which of the working directories in error status to
        /// immediately delete. Refuses directories with different
        /// status than `error`.
        ids: Vec<WorkingDirectoryId>,
    },
    /// Open the log file for the last run in a working directory in
    /// the `PAGER` (or `less`)
    Log {
        /// The ID of the working direcory for which to show the last
        /// log file
        id: WorkingDirectoryId,
    },
    /// Mark the given working directories for examination, so that
    /// they are not deleted by `evobench-run wd cleanup`
    Mark {
        /// The IDs of the working direcories to mark
        ids: Vec<WorkingDirectoryId>,
    },
    /// Change the status of the given working directories back to
    /// "error", so that they are again deleted by `evobench-run wd
    /// cleanup`
    Unmark {
        /// The IDs of the working direcories to unmark
        ids: Vec<WorkingDirectoryId>,
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

        /// The ID of the working directory to mark and enter
        id: WorkingDirectoryId,
    },
}

#[derive(Debug, clap::Subcommand)]
enum WdSubCommandCleanupMode {
    /// Delete all working directories with errors
    All,
    /// Delete those that were set aside at least the given number of
    /// days ago
    StaleForDays {
        /// Number of days (can be a floating point value)
        x: f32,
    },
}

enum RunResult {
    OnceResult(bool),
    NeedReExec(PathBuf),
}

/// Run through the queues forever unless `once` is true (in which
/// case it returns whether a job was run), but pick up config
/// changes; it also returns in non-once mode if the binary changes
/// and true was given for `restart_on_upgrades`.
fn run_queues(
    config_path: Option<PathBuf>,
    mut config_with_reload: RunConfigWithReload,
    mut queues: RunQueues,
    working_directory_base_dir: WorkingDirectoryPoolBaseDir,
    mut working_directory_pool: WorkingDirectoryPool,
    global_app_state_dir: &GlobalAppStateDir,
    once: bool,
    restart_on_upgrades: bool,
) -> Result<RunResult> {
    let opt_binary_and_mtime = if restart_on_upgrades {
        let path = std::env::current_exe()?;
        match path.metadata() {
            Ok(m) => {
                if let Ok(mtime) = m.modified() {
                    Some((path, mtime))
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let mut run_context = RunContext::default();
    let mut last_config_reload_error = None;
    let versioned_dataset_dir = VersionedDatasetDir::new();

    // Test-run
    if let Some(versioned_dataset_base_dir) =
        &config_with_reload.run_config.versioned_datasets_base_dir
    {
        debug!("Test-running versioned dataset search");

        let working_directory_id;
        {
            let mut pool = working_directory_pool.lock_mut()?;
            working_directory_id = pool.get_first()?;
            pool.clear_current_working_directory()?;
        }
        debug!("Got working directory {working_directory_id:?}");
        let ((), token) = working_directory_pool.process_in_working_directory(
            working_directory_id,
            &DateTimeWithOffset::now(),
            |working_directory| -> Result<()> {
                let working_directory = working_directory.into_inner().expect("still there");

                // Avoid the risk of an old working directory having
                // an older HEAD than all dataset versions.
                working_directory
                    .git_working_dir
                    .git(&["fetch", "--tags"], true)?;

                // XX capture all errors and return as Ok? Or is it OK
                // to re-clone the repo on all such errors?
                let head_commit_str = working_directory
                    .git_working_dir
                    .git_rev_parse("HEAD", true)?
                    .ok_or_else(|| anyhow!("can't resolve HEAD"))?;
                let head_commit: GitHash = head_commit_str.parse().map_err(|e| {
                    anyhow!(
                        "parsing commit id from HEAD from polling working dir: \
                         {head_commit_str:?}: {e}"
                    )
                })?;
                let lock = versioned_dataset_dir
                    .updated_git_graph(&working_directory.git_working_dir, &head_commit)?;

                for dataset_name_entry in std::fs::read_dir(&versioned_dataset_base_dir)
                    .map_err(ctx!("can't open directory {versioned_dataset_base_dir:?}"))?
                {
                    let dataset_name_entry = dataset_name_entry?;
                    let dataset_name = dataset_name_entry.file_name();
                    let dataset_name_str = dataset_name.to_str().ok_or_else(|| {
                        anyhow!("can't decode entry {:?}", dataset_name_entry.path())
                    })?;
                    let x =
                        lock.dataset_dir_for_commit(&versioned_dataset_base_dir, dataset_name_str)?;
                    debug!(
                        "Test-run of versioned dataset search for HEAD commit {head_commit_str} \
                     gave path: {x:?}"
                    );
                }
                Ok(())
            },
            None,
            "test-running versioned dataset search",
            None,
        )?;
        working_directory_pool.working_directory_cleanup(token)?;
    }

    loop {
        // XX handle errors without exiting? Or do that above

        let run_config = &config_with_reload.run_config;
        let output_base_dir = &run_config.output_base_dir;

        let queues_data = queues.data()?;

        let ran = queues_data.run_next_job(
            JobRunner {
                working_directory_pool: &mut working_directory_pool,
                output_base_dir: &output_base_dir,
                timestamp: DateTimeWithOffset::now(),
                run_config,
                versioned_dataset_dir: &versioned_dataset_dir,
            },
            &mut run_context,
        )?;

        if once {
            return Ok(RunResult::OnceResult(ran));
        }

        // XX have something better than polling?
        thread::sleep(Duration::from_secs(1));

        // Has our binary been updated?
        if let Some((binary, mtime)) = &opt_binary_and_mtime {
            if let Ok(metadata) = binary.metadata() {
                if let Ok(new_mtime) = metadata.modified() {
                    // if new_mtime > *mtime {  ? or allow downgrades, too:
                    if new_mtime != *mtime {
                        info!(
                            "this binary at {binary:?} has updated, \
                             from {} to {}, going to re-exec",
                            SystemTimeWithDisplay(*mtime),
                            SystemTimeWithDisplay(new_mtime)
                        );
                        return Ok(RunResult::NeedReExec(binary.to_owned()));
                    }
                }
            }
        }

        match config_with_reload.perhaps_reload_config(config_path.as_ref()) {
            Ok(Some(new_config_with_reload)) => {
                last_config_reload_error = None;
                // XX only if changed
                info!("reloaded configuration, re-initializing");
                drop(queues);
                drop(working_directory_pool);
                config_with_reload = new_config_with_reload;
                let conf = &config_with_reload.run_config;
                // XX handle errors without exiting? Or do that above
                queues = RunQueues::open(conf.queues.clone_arc(), true, &global_app_state_dir)?;
                working_directory_pool = WorkingDirectoryPool::open(
                    conf.working_directory_pool.clone_arc(),
                    working_directory_base_dir.clone(),
                    conf.remote_repository.url.clone(),
                    true,
                )?;
            }
            Ok(None) => {
                last_config_reload_error = None;
            }
            Err(e) => {
                let e_str = format!("{e:#?}");
                if let Some(last_e_str) = &last_config_reload_error {
                    if *last_e_str != e_str {
                        info!("note: attempting to reload configuration yielded error: {e_str}");
                        last_config_reload_error = Some(e_str);
                    }
                }
            }
        }
    }
}

const TARGET_NAME_WIDTH: usize = 14;

fn run() -> Result<Option<PathBuf>> {
    let Opts {
        log_level,
        config,
        subcommand,
    } = Opts::parse();

    set_log_level(log_level.try_into()?);

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
            return Ok(None);
        }
        _ => (),
    }

    let config_with_reload =
        RunConfigWithReload::load(config.as_ref(), |msg| bail!("need a config file, {msg}"))?;

    let conf = &config_with_reload.run_config;

    let global_app_state_dir = GlobalAppStateDir::new()?;

    let working_directory_base_dir =
        WorkingDirectoryPoolBaseDir::new(&conf.working_directory_pool, &|| {
            global_app_state_dir.working_directory_pool_base()
        })?;

    let queues = RunQueues::open(conf.queues.clone_arc(), true, &global_app_state_dir)?;

    match subcommand {
        SubCommand::ConfigFormats => unreachable!("already dispatched above"),

        SubCommand::ConfigSave { output_path } => {
            save_config_file(&output_path, &*config_with_reload.config_file)?;
        }

        SubCommand::ListAll {
            terminal_table_opts,
        } => {
            let already_inserted = open_already_inserted(&global_app_state_dir)?;

            let mut flat_jobs: Vec<(BenchmarkingJobParameters, SystemTime)> = Vec::new();
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
                &[38, 43, TARGET_NAME_WIDTH],
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
                        text: Cow::Borrowed("Target name"),
                        span: 1,
                    },
                    TerminalTableTitle {
                        text: Cow::Borrowed("Custom parameters"),
                        span: 1,
                    },
                ],
                None,
                terminal_table_opts,
                stdout().lock(),
            )?;
            for (params, insertion_time) in flat_jobs {
                let t = system_time_to_rfc3339(insertion_time);
                let BenchmarkingJobParameters {
                    run_parameters,
                    command,
                } = params;
                let RunParameters {
                    commit_id,
                    custom_parameters,
                } = &*run_parameters;
                let BenchmarkingCommand {
                    target_name,
                    subdir: _,
                    command: _,
                    arguments: _,
                } = &*command;

                let values: &[&dyn Display] =
                    &[&t, &commit_id, &target_name.as_str(), &custom_parameters];
                table.write_data_row(values, None)?;
            }
            drop(table.finish()?);
        }

        SubCommand::List {
            verbose,
            terminal_table_opts,
            all,
        } => {
            fn table_with_titles<'v, 's, O: Write + IsTerminal>(
                titles: &'s [TerminalTableTitle],
                style: Option<Style>,
                terminal_table_opts: &TerminalTableOpts,
                out: O,
                verbose: bool,
            ) -> Result<TerminalTable<'v, 's, O>> {
                let insertion_time_width = if verbose { 82 } else { 37 };
                TerminalTable::start(
                    // t                    R pr WD reason commit target
                    &[insertion_time_width, 3, 6, 3, 25, 42, TARGET_NAME_WIDTH],
                    titles,
                    style,
                    terminal_table_opts.clone(),
                    out,
                )
            }

            let mut out = stdout().lock();

            let full_span;
            {
                // Show a table with no data rows, for main titles
                let titles = &[
                    "Insertion_time",
                    "S", // Status
                    "Prio",
                    "WD",
                    "Reason",
                    "Commit_id",
                    "Target_name",
                    "Custom_parameters",
                ]
                // .map() is not const
                .map(|s| TerminalTableTitle {
                    text: Cow::Borrowed(s),
                    span: 1,
                });
                full_span = titles.len();
                // Somehow have to move `out` in and out, `&mut out`
                // would not satisfy IsTerminal.
                let table = table_with_titles(
                    titles,
                    // Note: in spite of `TERM=xterm-256color`, `watch
                    // --color` still only supports system colors
                    // 0..14!  (Can still not use `.rgb(10, 70, 140)`
                    // nor `.fg(Color::Fixed(30))`, and watch 4.0.2
                    // does not support `TERM=xterm-truecolor`.)
                    Some(Style::new().fg(Color::Fixed(4)).italic().bold()),
                    &terminal_table_opts,
                    out,
                    verbose,
                )?;
                out = table.finish()?;
            }

            let now = SystemTime::now();
            let show_queue =
                |i: &str, run_queue: &RunQueue, is_extra_queue: bool, out| -> Result<_> {
                    let RunQueue {
                        file_name,
                        schedule_condition,
                        queue,
                    } = run_queue;

                    // "Insertion time"
                    // "R", "E", ""
                    // priority
                    // reason
                    // "Commit id"
                    // "Custom parameters"
                    let titles = &[TerminalTableTitle {
                        text: format!(
                            "{i}: queue {:?} ({schedule_condition}):",
                            file_name.as_str()
                        )
                        .into(),
                        span: full_span,
                    }];
                    let mut table =
                        table_with_titles(titles, None, &terminal_table_opts, out, verbose)?;

                    // We want the last view_jobs_max_len items, one more
                    // if that's the complete list (the additional entry
                    // then occupying the "entries skipped" line). Don't
                    // want to collect the whole list first (leads to too
                    // many open filehandles), don't want to go through it
                    // twice (once for counting, once to skip); getting
                    // them in reverse, taking the first n, collecting,
                    // then reversing the list would be one way, but
                    // cleaner is to use a two step approach, first get
                    // the sorted collection of keys (cheap to hold in
                    // memory and needs to be retrieved underneath
                    // anyway), get the section we want, then use
                    // resolve_entries to load the items still in
                    // streaming fashion.  Note: this could show fewer
                    // than limit items even after showing "skipped",
                    // because items can vanish between getting
                    // sorted_keys and resolve_entries. But that is really
                    // no big deal.
                    let limit = if is_extra_queue && !all {
                        // Get 2 more since showing "skipped 1 entry" is
                        // not economic, and we just look at number 0
                        // after subtracting, i.e. include the equal case.
                        conf.queues.view_jobs_max_len + 2
                    } else {
                        usize::MAX
                    };
                    let all_sorted_keys = queue.sorted_keys(false, None, false)?;
                    let shown_sorted_keys;
                    if let Some(num_skipped_2) = all_sorted_keys.len().checked_sub(limit) {
                        let num_skipped = num_skipped_2 + 2;
                        table.print(&format!("... ({num_skipped} entries skipped)\n"))?;
                        shown_sorted_keys = &all_sorted_keys[num_skipped..];
                    } else {
                        shown_sorted_keys = &all_sorted_keys;
                    }
                    for entry in queue.resolve_entries(shown_sorted_keys.into()) {
                        let mut entry = entry?;
                        let file_name = get_filename(&entry)?;
                        let key = entry.key()?;
                        let job = entry.get()?;
                        let commit_id = &*job
                            .benchmarking_job_public
                            .run_parameters
                            .commit_id
                            .to_string();
                        let reason = if let Some(reason) = &job.benchmarking_job_public.reason {
                            reason.as_ref()
                        } else {
                            ""
                        };
                        let opt_current_working_directory =
                            working_directory_base_dir.get_current_working_directory()?;
                        let custom_parameters = &*job
                            .benchmarking_job_public
                            .run_parameters
                            .custom_parameters
                            .to_string();
                        let (locking, is_locked) = if schedule_condition.is_inactive() {
                            ("", false)
                        } else {
                            let lock_status = entry
                                .take_lockable_file()
                                .expect("not taken before")
                                .lock_status()?;
                            if lock_status == LockStatus::ExclusiveLock {
                                let s = if let Some(dir) = opt_current_working_directory {
                                    let status = working_directory_base_dir
                                        .get_working_directory_status(dir)?;
                                    match status.status {
                                        // CheckedOut wasn't planned
                                        // to happen, but now happens
                                        // for new working dir
                                        // assignment
                                        Status::CheckedOut => "R0",
                                        Status::Processing => "R", // running
                                        Status::Error => "F",      // failure
                                        Status::Finished => "E",   // evaluating
                                        Status::Examination => "X", // manually marked
                                    }
                                } else {
                                    "R"
                                };
                                (s, true)
                            } else {
                                ("", false)
                            }
                        };
                        let priority = &*job.priority()?.to_string();
                        let wd = if is_locked {
                            opt_current_working_directory
                                .map(|v| v.to_number_string())
                                .unwrap_or_else(|| "".into())
                        } else {
                            job.benchmarking_job_state
                                .last_working_directory
                                .map(|v| v.to_number_string())
                                .unwrap_or_else(|| "".into())
                        };
                        let target_name = job.benchmarking_job_public.command.target_name.as_str();

                        let system_time = key.system_time();
                        let is_older = {
                            let age = now.duration_since(system_time)?;
                            age > Duration::from_secs(3600 * 24)
                        };
                        let time = if verbose {
                            &*format!("{file_name} ({key})")
                        } else {
                            let datetime: DateTime<Local> = system_time.into();
                            &*datetime.to_rfc3339()
                        };
                        table.write_data_row(
                            &[
                                time,
                                locking,
                                priority,
                                &*wd,
                                reason,
                                commit_id,
                                target_name,
                                custom_parameters,
                            ],
                            if is_older {
                                // Note: need `TERM=xterm-256color`
                                // for `watch --color` to not turn
                                // this color to black!
                                Some(Style::new().bright_black())
                            } else {
                                None
                            },
                        )?;
                        if verbose {
                            let s = ron_to_string_pretty(&job)?;
                            table.print(&format!("{s}\n\n"))?;
                        }
                    }
                    Ok(table.finish()?)
                };

            let width = get_terminal_width(1);
            let bar_of = |c: &str| c.repeat(width);
            let (thin_bar, thick_bar) = if *UNICODE_IS_FINE {
                (bar_of("─"), bar_of("═"))
            } else {
                (bar_of("-"), bar_of("="))
            };

            for (i, run_queue) in queues.pipeline().iter().enumerate() {
                println!("{thin_bar}");
                out = show_queue(&(i + 1).to_string(), run_queue, false, out)?;
            }
            println!("{thick_bar}");
            let perhaps_show_extra_queue = |queue_name: &str,
                                            queue_field: &str,
                                            run_queue: Option<&RunQueue>,
                                            mut out|
             -> Result<_> {
                if let Some(run_queue) = run_queue {
                    out = show_queue(queue_name, run_queue, true, out)?;
                } else {
                    println!("No {queue_field} is configured")
                }
                Ok(out)
            };
            out =
                perhaps_show_extra_queue("done", "done_jobs_queue", queues.done_jobs_queue(), out)?;
            println!("{thin_bar}");
            _ = perhaps_show_extra_queue(
                "failures",
                "erroneous_jobs_queue",
                queues.erroneous_jobs_queue(),
                out,
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
                    &conf.job_templates_for_insert,
                )?,
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
                    &conf.job_templates_for_insert,
                )?,
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
                    &conf.remote_repository.remote_branch_names_for_poll,
                )?
            };
            let num_commits = commits.len();

            let mut benchmarking_jobs = Vec::new();
            for (branch_name, commit_id, job_templates) in commits {
                let opts = BenchmarkingJobOpts {
                    reason: BenchmarkingJobReasonOpt {
                        reason: branch_name.as_str().to_owned().into(),
                    },
                    benchmarking_job_settings: (*conf.benchmarking_job_settings).clone(),
                    run_parameters: RunParametersOpts { commit_id },
                };
                benchmarking_jobs.append(&mut opts.complete_jobs(
                    // already using conf.benchmarking_job_settings from above
                    None,
                    &job_templates,
                )?);
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

        SubCommand::Run { mode } => {
            let run_lock_path = conf.run_jobs_lock_path(&global_app_state_dir);
            // Should StandaloneExclusiveFileLock have an option to
            // create itself?
            let _ = std::fs::write(&run_lock_path, "");
            let _run_lock = StandaloneExclusiveFileLock::try_lock_path(run_lock_path, || {
                "getting the global lock for running jobs".into()
            })?;

            let working_directory_pool = WorkingDirectoryPool::open(
                conf.working_directory_pool.clone_arc(),
                working_directory_base_dir.clone(),
                conf.remote_repository.url.clone(),
                true,
            )?;

            match mode {
                RunMode::One { false_if_none } => {
                    match run_queues(
                        config,
                        config_with_reload,
                        queues,
                        working_directory_base_dir,
                        working_directory_pool,
                        &global_app_state_dir,
                        true,
                        false,
                    )? {
                        RunResult::OnceResult(ran) => {
                            if false_if_none {
                                exit(if ran { 0 } else { 1 })
                            }
                        }
                        RunResult::NeedReExec(_) => unreachable!(),
                    }
                }
                RunMode::Daemon {
                    action,
                    restart_on_upgrades,
                } => {
                    if let Some(action) = action {
                        todo!("daemonization {action:?}")
                    } else {
                        match run_queues(
                            config,
                            config_with_reload,
                            queues,
                            working_directory_base_dir,
                            working_directory_pool,
                            &global_app_state_dir,
                            false,
                            restart_on_upgrades,
                        )? {
                            RunResult::OnceResult(_) => unreachable!(),
                            RunResult::NeedReExec(executable_path) => {
                                return Ok(Some(executable_path))
                            }
                        }
                    }
                }
            }
        }

        SubCommand::Wd { subcommand } => {
            // XX COPYPASTE
            let mut working_directory_pool = WorkingDirectoryPool::open(
                conf.working_directory_pool.clone_arc(),
                working_directory_base_dir.clone(),
                conf.remote_repository.url.clone(),
                true,
            )?;
            // /COPYPASTE

            let check_original_status = |wd: &WorkingDirectory,
                                         allow_access: bool,
                                         allowed_statuses: &str|
             -> Result<Status> {
                let status = wd.working_directory_status.status;
                if status.can_be_used_for_jobs() && !allow_access {
                    bail!(
                        "this action is only for working directories in {allowed_statuses} \
                         status, but directory {} has status '{}'",
                        wd.parent_path_and_id()?.1,
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

            let mut do_mark = |wanted_status: Status, id| -> Result<Option<Status>> {
                let mut guard = working_directory_pool.lock_mut()?;
                if let Some(mut wd) = guard.get_working_directory_mut(id) {
                    let original_status = check_original_status(&*wd, false, "error/examination")
                        .map_err(ctx!("refusing working directory {id}"))?;
                    wd.set_and_save_status(wanted_status)?;
                    Ok(Some(original_status))
                } else {
                    Ok(None)
                }
            };

            match subcommand {
                WdSubCommand::List {
                    terminal_table_opts,
                    active,
                    error,
                    sort_used,
                    id_only,
                } => {
                    let mut all_entries: Vec<_> = working_directory_pool.all_entries().collect();

                    if sort_used {
                        all_entries.sort_by(|a, b| a.1.last_use.cmp(&b.1.last_use))
                    }

                    let titles = &["id", "status", "num_runs", "creation_timestamp", "last_use"]
                        .map(|s| TerminalTableTitle {
                            text: Cow::Borrowed(s),
                            span: 1,
                        });

                    let mut table = if id_only {
                        None
                    } else {
                        Some(TerminalTable::start(
                            &[3 + 2, Status::MAX_STR_LEN + 2, 8 + 2, 35 + 2],
                            titles,
                            None,
                            terminal_table_opts,
                            stdout().lock(),
                        )?)
                    };

                    for (id, wd) in &all_entries {
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
                                table.write_data_row(
                                    &[
                                        id.to_number_string(),
                                        status.to_string(),
                                        num_runs.to_string(),
                                        creation_timestamp.to_string(),
                                        system_time_to_rfc3339(wd.last_use),
                                    ],
                                    None,
                                )?;
                            } else {
                                println!("{id}");
                            }
                        }
                    }

                    if let Some(table) = table {
                        let _ = table.finish()?;
                    }
                }
                WdSubCommand::Cleanup {
                    dry_run,
                    verbose,
                    mode,
                } => {
                    let stale_days = match mode {
                        WdSubCommandCleanupMode::All => 0.,
                        WdSubCommandCleanupMode::StaleForDays { x } => x,
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
                            cleanup_ids.push(*id);
                        }
                    }

                    {
                        let mut lock = working_directory_pool.lock_mut()?;
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
                }
                WdSubCommand::Delete {
                    dry_run,
                    verbose,
                    ids,
                } => {
                    {
                        let mut lock = working_directory_pool.lock_mut()?;
                        for id in ids {
                            let wd = lock
                                .get_working_directory_mut(id)
                                .ok_or_else(|| anyhow!("working directory {id} does not exist"))?;
                            let status = wd.working_directory_status.status;
                            if status != Status::Error {
                                let tip = if status == Status::Examination {
                                    "; please first use the `unmark` action to move it \
                                     out of examination"
                                } else {
                                    ""
                                };
                                bail!(
                                    "working directory {id} is not in `error`, but `{status}` \
                                     status{tip}"
                                );
                            }
                            if dry_run {
                                let path = wd.git_working_dir.working_dir_path_ref();
                                eprintln!("would delete working directory at {path:?}");
                            } else {
                                // XX Note: can this fail if a concurrent
                                // instance deletes it in the mean time?
                                lock.delete_working_directory(id)?;
                                if verbose {
                                    println!("{id}");
                                }
                            }
                        }
                    }
                }
                WdSubCommand::Log { id } => {
                    let no_exist = || anyhow!("there is no working directory for id {id}");
                    let working_directory = working_directory_pool
                        .get_working_directory(id)
                        .ok_or_else(&no_exist)?;

                    check_original_status(working_directory, true, "non-finished")?;

                    let (standard_log_path, _id) =
                        working_directory.last_standard_log_path()?.ok_or_else(|| {
                            anyhow!("could not find a log file for working directory {id}")
                        })?;

                    let pager = match std::env::var("PAGER") {
                        Ok(s) => s,
                        Err(e) => match e {
                            std::env::VarError::NotPresent => "less".into(),
                            _ => bail!("can't decode PAGER env var: {e}"),
                        },
                    };

                    let mut cmd = Command::new(&pager);
                    cmd.arg(standard_log_path);
                    return Err(cmd.exec()).with_context(|| anyhow!("executing pager {pager:?}"));
                }
                WdSubCommand::Mark { ids } => {
                    for id in ids {
                        if do_mark(Status::Examination, id)?.is_none() {
                            warn!("there is no working directory for id {id}");
                        }
                    }
                }
                WdSubCommand::Unmark { ids } => {
                    for id in ids {
                        if do_mark(Status::Error, id)?.is_none() {
                            warn!("there is no working directory for id {id}");
                        }
                    }
                }
                WdSubCommand::Enter { mark, unmark, id } => {
                    if mark && unmark {
                        bail!("please only give one of the --mark or --unmark options")
                    }

                    let no_exist = || anyhow!("there is no working directory for id {id}");
                    let original_status =
                        do_mark(Status::Examination, id)?.ok_or_else(&no_exist)?;

                    let working_directory = working_directory_pool
                        .get_working_directory(id)
                        .ok_or_else(&no_exist)?;

                    let (standard_log_path, _id) =
                        working_directory.last_standard_log_path()?.ok_or_else(|| {
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
                    } = &*command;

                    let mut vars: Vec<(&str, &OsStr)> = custom_parameters
                        .btree_map()
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_ref()))
                        .collect();

                    let commit_id_str = commit_id.to_string();
                    vars.push(("COMMIT_ID", &commit_id_str.as_ref()));

                    let versioned_dataset_dir = VersionedDatasetDir::new();
                    let dataset_dir_;
                    if let Some(dataset_dir) = dataset_dir_for(
                        conf.versioned_datasets_base_dir.as_deref(),
                        &custom_parameters,
                        &versioned_dataset_dir,
                        &working_directory.git_working_dir,
                        &commit_id,
                    )? {
                        dataset_dir_ = dataset_dir;
                        vars.push(("DATASET_DIR", &dataset_dir_.as_ref()));
                    }

                    let exports = vars
                        .iter()
                        .map(|(k, v)| {
                            bash_export_variable_string(k, &v.to_string_lossy(), "  ", "\n")
                        })
                        .join("");

                    let shell = std::env::var_os("SHELL").unwrap_or("bash".into());

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
                        bash_string_from_program_path_and_args(command, arguments)?
                    );

                    let actual_commit = working_directory.git_working_dir.get_head_commit_id()?;
                    if commit_id_str != actual_commit {
                        println!(
                            "*** WARNING: the checked-out commit in this directory \
                             does not match the commit id for the job! ***\n"
                        );
                    }

                    // Enter dir without any locking (other than dir
                    // being in Status::Examination now), OK?

                    let mut cmd = Command::new(&shell);
                    cmd.envs(vars);
                    cmd.current_dir(
                        working_directory
                            .git_working_dir
                            .working_dir_path_ref()
                            .append(subdir),
                    );
                    let status = cmd.status()?;

                    if unmark || original_status != Status::Examination {
                        if mark {
                            // keep marked
                        } else {
                            let do_revert = unmark
                                || ask_yn(&format!(
                                    "Should the working directory status be reverted to \
                                     '{original_status}' (i.e. are you done)?"
                                ))?;

                            if do_revert {
                                let mut wd = working_directory_pool
                                    .lock_mut()?
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
                        }
                    } else {
                        if !mark {
                            println!("Leaving working directory status at 'examination'");
                        }
                    }

                    exit(status.to_exit_code());
                }
            }
        }
    }

    Ok(None)
}

fn main() -> Result<()> {
    if let Some(executable_path) = run()? {
        Err(re_exec_with_existing_args_and_env(executable_path))?
    } else {
        Ok(())
    }
}
