use anyhow::{Context, Result, anyhow, bail};
use chj_unix_util::{
    daemon::{
        Daemon, DaemonCheckExit, DaemonMode, DaemonOpts, ExecutionResult,
        warrants_restart::{
            RestartForConfigChangeOpts, RestartForExecutableChangeOpts,
            RestartForExecutableOrConfigChange,
        },
    },
    logging::{TimestampMode, TimestampOpts},
    polling_signals::PollingSignals,
    timestamp_formatter::TimestampFormatter,
};
use chrono::{DateTime, Local};
use cj_path_util::path_util::AppendToPath;
use clap::Parser;
use itertools::Itertools;
use lazy_static::lazy_static;
use yansi::{Color, Style};

use std::{
    borrow::Cow,
    env,
    ffi::OsStr,
    fmt::Display,
    io::{BufWriter, IsTerminal, Write, stderr, stdout},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, exit},
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, SystemTime},
};

use evobench_tools::{
    ask::ask_yn,
    config_file::{self, ConfigFile, ron_to_string_pretty, save_config_file},
    ctx, debug,
    get_terminal_width::get_terminal_width,
    git::GitHash,
    info,
    io_utils::bash::{bash_export_variable_string, bash_string_from_program_path_and_args},
    key::{BenchmarkingJobParameters, RunParameters, RunParametersOpts},
    key_val_fs::key_val::Entry,
    lazyresult,
    lockable_file::{LockStatus, StandaloneExclusiveFileLock, StandaloneFileLockError},
    run::{
        benchmarking_job::{BenchmarkingJobOpts, BenchmarkingJobReasonOpt},
        command_log_file::CommandLogFile,
        config::{BenchmarkingCommand, RunConfig, RunConfigBundle, RunConfigOpts},
        dataset_dir_env_var::dataset_dir_for,
        env_vars::assert_evobench_env_var,
        global_app_state_dir::GlobalAppStateDir,
        insert_jobs::{ForceOpt, QuietOpt, insert_jobs, open_already_inserted},
        polling_pool::PollingPool,
        run_context::RunContext,
        run_job::{JobRunner, get_commit_tags},
        run_queue::RunQueue,
        run_queues::RunQueues,
        sub_command::insert::Insert,
        versioned_dataset_dir::VersionedDatasetDir,
        working_directory::{FetchedTags, Status, WorkingDirectory, WorkingDirectoryStatus},
        working_directory_pool::{
            WorkingDirectoryId, WorkingDirectoryPool, WorkingDirectoryPoolAndLock,
            WorkingDirectoryPoolBaseDir,
        },
    },
    serde::date_and_time::{DateTimeWithOffset, system_time_to_rfc3339},
    terminal_table::{TerminalTable, TerminalTableOpts, TerminalTableTitle},
    utillib::{
        arc::CloneArc,
        logging::{LOG_LOCAL_TIME, LogLevel, LogLevelOpt, set_log_level},
        unix::ToExitCode,
    },
    warn,
};

type CheckExit<'t> =
    DaemonCheckExit<'t, RestartForExecutableOrConfigChange<Arc<ConfigFile<RunConfigOpts>>>>;

const DEFAULT_RESTART_ON_UPGRADES: bool = true;
const DEFAULT_RESTART_ON_CONFIG_CHANGE: bool = true;

lazy_static! {
    static ref UNICODE_IS_FINE: bool = (|| -> Option<bool> {
        let term = env::var_os("TERM")?;
        let lang = env::var_os("LANG")?;
        let lang = lang.to_str()?;
        Some(term.as_bytes().starts_with(b"xterm") && lang.contains("UTF-8"))
    })()
    .unwrap_or(false);
}

#[derive(clap::Parser, Debug)]
#[clap(next_line_help = true)]
#[clap(set_term_width = get_terminal_width(4))]
/// Schedule and query benchmarking jobs.
struct Opts {
    #[clap(flatten)]
    log_level: LogLevelOpt,

    /// Override the path to the config file (default: the paths
    /// `~/.evobench-jobs.*` where a single one exists where the `*` is
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

    /// Manually insert (a) new job(s)
    Insert {
        /// Choice of how to specify the job parameters
        #[clap(subcommand)]
        insert: Insert,
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

        #[clap(subcommand)]
        mode: RunMode,
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
        subcommand: Wd,
    },
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum RunMode {
    /// Run the single job that is first due.
    One {
        /// Exit with code 1 if there is no runnable job / there were
        /// no jobs to insert.
        #[clap(long)]
        false_if_none: bool,
    },
    /// Run forever, until terminated (note: evobench-jobs uses
    /// --restart-on-failures by default)
    Daemon {
        #[clap(flatten)]
        opts: DaemonOpts,
        #[clap(flatten)]
        restart_for_executable_change_opts: RestartForExecutableChangeOpts,
        #[clap(flatten)]
        restart_for_config_change_opts: RestartForConfigChangeOpts,

        /// The logging level while running as daemon (overrides the
        /// top-level logging options like --verbose, --debug,
        /// --quiet)
        #[clap(short, long, default_value = "info")]
        log_level: LogLevel,

        /// Whether to run in the foreground, or start or stop a
        /// daemon running in the background (or report the status
        /// about it). Give `help` to see the options. evobench-jobs
        /// defaults to the 'hard' actions.
        action: DaemonMode,
    },
}

#[derive(Debug, clap::Subcommand)]
enum Wd {
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

        /// Which of the working directories in error status to
        /// immediately delete. Refuses directories with different
        /// status than `error` unless `--force` was given.
        ids: Vec<WorkingDirectoryId>,
    },
    /// Open the log file for the last run in a working directory in
    /// the `PAGER` (or `less`)
    Log {
        /// Instead of opening the last log file, show a list of all
        /// log files for the given working directory, sorted by run
        /// start time (newest at the bottom)
        #[clap(short, long)]
        list: bool,

        /// The ID of the working direcory for which to show the (last)
        /// log file(s)
        id: WorkingDirectoryId,
    },
    /// Mark the given working directories for examination, so that
    /// they are not deleted by `evobench-jobs wd cleanup`
    Mark {
        /// The IDs of the working direcories to mark
        ids: Vec<WorkingDirectoryId>,
    },
    /// Change the status of the given working directories back to
    /// "error", so that they are again deleted by `evobench-jobs wd
    /// cleanup`
    Unmark {
        /// The IDs of the working direcories to unmark
        ids: Vec<WorkingDirectoryId>,
    },
    /// Change the status of the given working directories back to
    /// "checkedout", so that they can be used again by `evobench-jobs
    /// run`. (Be careful that you don't recycle dirs with problems
    /// that lead to errors again. It may be safer, albeit costlier,
    /// to `delete` the dirs instead.)
    Recycle {
        /// The IDs of the working direcories to recycle
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

        /// Force entering even when the status is `processing`; in this case,
        #[clap(long)]
        force: bool,

        /// Do not run `git fetch --tags` inside the working directory
        /// (usually it's a good idea to run it, to ensure the dataset
        /// dir and `COMMIT_TAGS` are chosen based on up to date
        /// remote data)
        #[clap(long)]
        no_fetch: bool,

        /// The ID of the working directory to mark and enter
        id: WorkingDirectoryId,
    },
}

#[derive(Debug, clap::Subcommand)]
enum WdCleanupMode {
    /// Delete all working directories with errors
    All,
    /// Delete those that were set aside at least the given number of
    /// days ago
    StaleForDays {
        /// Number of days (can be a floating point value)
        x: f32,
    },
}

fn open_working_directory_pool(
    conf: &RunConfig,
    working_directory_base_dir: Arc<WorkingDirectoryPoolBaseDir>,
    create_dir_if_not_exists: bool,
) -> Result<WorkingDirectoryPoolAndLock> {
    WorkingDirectoryPool::open(
        conf.working_directory_pool.clone_arc(),
        working_directory_base_dir,
        conf.remote_repository.url.clone(),
        create_dir_if_not_exists,
    )
}

fn open_working_directory_change_signals(conf: &RunConfig) -> Result<PollingSignals> {
    let signals_path = conf.working_directory_change_signals_path();
    PollingSignals::open(&signals_path, 0).map_err(ctx!("opening signals path {signals_path:?}"))
}

enum RunResult {
    /// In one-job mode, indicates whether it ran any job
    OnceResult(bool),
    /// In daemon mode
    StopOrRestart,
}

/// Run through the queues forever unless `once` is true (in which
/// case it returns whether a job was run), but pick up config
/// changes; it also returns in non-once mode if the binary changes
/// and true was given for `restart_on_upgrades`.
fn run_queues<'ce>(
    run_config_bundle: RunConfigBundle,
    queues: RunQueues,
    working_directory_base_dir: Arc<WorkingDirectoryPoolBaseDir>,
    mut working_directory_pool: WorkingDirectoryPool,
    once: bool,
    daemon_check_exit: Option<CheckExit<'ce>>,
) -> Result<RunResult> {
    let _run_lock = get_run_lock(&run_config_bundle.run_config)?;

    let mut run_context = RunContext::default();
    let versioned_dataset_dir = VersionedDatasetDir::new();

    // Test-run
    if let Some(versioned_dataset_base_dir) =
        &run_config_bundle.run_config.versioned_datasets_base_dir
    {
        debug!("Test-running versioned dataset search");

        let working_directory_id;
        {
            let mut pool = working_directory_pool.lock_mut("evobench-jobs::run_queues")?;
            working_directory_id = pool.get_first()?;
            pool.clear_current_working_directory()?;
        }
        debug!("Got working directory {working_directory_id:?}");
        let ((), token) = working_directory_pool.process_in_working_directory(
            working_directory_id,
            &DateTimeWithOffset::now(),
            |working_directory| -> Result<()> {
                let working_directory = working_directory.into_inner().expect("still there");

                // Fetch the tags so that comparing dataset versions
                // can work. (Avoid the risk of an old working
                // directory having an older HEAD than all dataset
                // versions.)
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
                         {head_commit_str:?}: {e:#}"
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

    let mut working_directory_change_signals =
        open_working_directory_change_signals(&run_config_bundle.run_config)?;

    loop {
        // XX handle errors without exiting? Or do that above

        let run_config = &run_config_bundle.run_config;
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

        if let Some(daemon_check_exit) = daemon_check_exit.as_ref() {
            if daemon_check_exit.want_exit() {
                return Ok(RunResult::StopOrRestart);
            }
        }

        // Do we need to re-initialize the working directory pool?
        if working_directory_change_signals.get_number_of_signals() > 0 {
            info!("the working directory pool was updated outside the app, reload it");
            let conf = &run_config_bundle.run_config;
            working_directory_pool =
                open_working_directory_pool(conf, working_directory_base_dir.clone_arc(), true)?
                    .into_inner();
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum GetRunLockError {
    #[error("{0}")]
    AlreadyLocked(StandaloneFileLockError),
    #[error("{0}")]
    Generic(anyhow::Error),
}

// omg the error handling.
fn get_run_lock(conf: &RunConfig) -> Result<StandaloneExclusiveFileLock, GetRunLockError> {
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

const TARGET_NAME_WIDTH: usize = 14;

const DEFAULT_IS_HARD: bool = true;

fn run() -> Result<Option<ExecutionResult>> {
    let Opts {
        log_level,
        config,
        subcommand,
    } = Opts::parse();

    set_log_level(log_level.try_into()?);
    // Interactive use should get local time. (Daemon mode overwrites
    // this.)
    LOG_LOCAL_TIME.store(true, Ordering::SeqCst);

    let config: Option<Arc<Path>> = config.map(Into::into);

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

    let run_config_bundle = RunConfigBundle::load(
        config,
        |msg| bail!("need a config file, {msg}"),
        GlobalAppStateDir::new()?,
    )?;

    let conf = &run_config_bundle.run_config;

    let working_directory_base_dir = Arc::new(WorkingDirectoryPoolBaseDir::new(
        &conf.working_directory_pool,
        &|| {
            run_config_bundle
                .global_app_state_dir
                .working_directory_pool_base()
        },
    )?);

    let open_queues = |run_config_bundle: &RunConfigBundle| -> Result<RunQueues> {
        RunQueues::open(
            run_config_bundle.run_config.queues.clone_arc(),
            true,
            &run_config_bundle.global_app_state_dir,
        )
    };
    let mut queues = lazyresult! {
        open_queues(&run_config_bundle)
    };

    match subcommand {
        SubCommand::ConfigFormats => unreachable!("already dispatched above"),

        SubCommand::ConfigSave { output_path } => {
            save_config_file(&output_path, &**run_config_bundle.config_file)?;
            Ok(None)
        }

        SubCommand::ListAll {
            terminal_table_opts,
        } => {
            let already_inserted = open_already_inserted(&run_config_bundle.global_app_state_dir)?;

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
                let t = system_time_to_rfc3339(insertion_time, true);
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
                    pre_exec_bash_code: _,
                } = &*command;

                let values: &[&dyn Display] =
                    &[&t, &commit_id, &target_name.as_str(), &custom_parameters];
                table.write_data_row(values, None)?;
            }
            drop(table.finish()?);
            Ok(None)
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
                    &[insertion_time_width, 3, 6, 5, 25, 42, TARGET_NAME_WIDTH],
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

            let lock = working_directory_base_dir.lock("for SubCommand::List show_queue")?;

            // Not kept in sync with what happens during for loop; but
            // then it is really about the status stored inside
            // `pool`, thus that doesn't even matter!
            let opt_current_working_directory = lock.read_current_working_directory()?;

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
                                .get_lock_status()?;
                            if lock_status == LockStatus::ExclusiveLock {
                                let s = if let Some(dir) = opt_current_working_directory {
                                    let status = lock.read_working_directory_status(dir)?;
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
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "".into())
                        } else {
                            job.benchmarking_job_state
                                .last_working_directory
                                .map(|v| v.to_string())
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

            let queues = queues.force()?;
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
            Ok(None)
        }

        SubCommand::Insert { insert } => {
            insert.run(&run_config_bundle, &mut queues)?;
            Ok(None)
        }

        SubCommand::Poll {
            force,
            quiet,
            no_fail,
            mode,
        } => {
            // Returns whether at least 1 job was inserted
            let mut try_run_poll = |daemon_check_exit: Option<CheckExit>| -> Result<bool> {
                loop {
                    let (commits, non_resolving) = {
                        let mut polling_pool = PollingPool::open(
                            &conf.remote_repository.url,
                            &run_config_bundle
                                .global_app_state_dir
                                .working_directory_for_polling_pool_base()?,
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
                    let queues = queues.force()?;
                    let n = insert_jobs(
                        benchmarking_jobs,
                        &run_config_bundle.global_app_state_dir,
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
                                println!(
                                    "inserted {n}/{n_original} jobs (for {num_commits} commits)"
                                );
                            }
                        }
                    } else {
                        bail!(
                            "inserted {n}/{n_original} jobs (for {num_commits} commits), \
                             but the following names did not resolve: {non_resolving:?}"
                        )
                    }

                    if let Some(daemon_check_exit) = &daemon_check_exit {
                        if daemon_check_exit.want_exit() {
                            return Ok(n >= 1);
                        }
                    } else {
                        return Ok(n >= 1);
                    }

                    std::thread::sleep(Duration::from_secs(15));
                }
            };

            match mode {
                RunMode::One { false_if_none } => {
                    let did_insert = try_run_poll(None)?;
                    if false_if_none && !did_insert {
                        exit(1);
                    }
                    Ok(None)
                }
                RunMode::Daemon {
                    opts,
                    restart_for_executable_change_opts,
                    restart_for_config_change_opts,
                    log_level,
                    action,
                } => {
                    let local_time = opts.logging_opts.local_time;

                    let run = |daemon_check_exit: CheckExit| -> Result<()> {
                        // Use the requested time setting for
                        // local time stamp generation, too (now
                        // the default is UTC, which is expected
                        // for a daemon).
                        LOG_LOCAL_TIME.store(local_time, Ordering::SeqCst);

                        set_log_level(log_level);

                        try_run_poll(Some(daemon_check_exit))?;
                        Ok(())
                    };

                    let config_file = run_config_bundle.config_file.clone_arc();
                    let other_restart_checks = restart_for_executable_change_opts
                        .to_restarter(
                            DEFAULT_RESTART_ON_UPGRADES,
                            TimestampFormatter {
                                use_rfc3339: true,
                                local_time,
                            },
                        )?
                        .and_config_change_opts(
                            restart_for_config_change_opts,
                            DEFAULT_RESTART_ON_CONFIG_CHANGE,
                            config_file,
                        );
                    let daemon = Daemon {
                        opts,
                        restart_on_failures_default: true,
                        restart_opts: None,
                        timestamp_opts: TimestampOpts {
                            use_rfc3339: true,
                            mode: TimestampMode::Automatic {
                                mark_added_timestamps: true,
                            },
                        },
                        paths: conf.polling_daemon.clone(),
                        other_restart_checks,
                        run,
                    };
                    let r = daemon.execute(action, DEFAULT_IS_HARD)?;
                    Ok(Some(r))
                }
            }
        }

        SubCommand::Run { mode } => {
            let open_working_directory_pool = |conf: &RunConfig,
                                               working_directory_base_dir: &Arc<
                WorkingDirectoryPoolBaseDir,
            >|
             -> Result<_> {
                Ok(
                    open_working_directory_pool(conf, working_directory_base_dir.clone(), true)?
                        .into_inner(),
                )
            };

            match mode {
                RunMode::One { false_if_none } => {
                    let queues = queues.into_value()?;
                    let working_directory_pool = open_working_directory_pool(
                        &run_config_bundle.run_config,
                        &working_directory_base_dir,
                    )?;
                    match run_queues(
                        run_config_bundle,
                        queues,
                        working_directory_base_dir,
                        working_directory_pool,
                        true,
                        None,
                    )? {
                        RunResult::OnceResult(ran) => {
                            if false_if_none {
                                exit(if ran { 0 } else { 1 })
                            } else {
                                Ok(None)
                            }
                        }
                        RunResult::StopOrRestart => unreachable!("only daemon mode issues this"),
                    }
                }
                RunMode::Daemon {
                    opts,
                    restart_for_executable_change_opts,
                    restart_for_config_change_opts,
                    log_level,
                    action,
                } => {
                    let paths = conf.run_jobs_daemon.clone();
                    let local_time = opts.logging_opts.local_time;
                    let config_file = run_config_bundle.config_file.clone_arc();
                    let run = |daemon_check_exit: CheckExit| -> Result<()> {
                        // Use the requested time setting for
                        // local time stamp generation, too (now
                        // the default is UTC, which is expected
                        // for a daemon).
                        LOG_LOCAL_TIME.store(local_time, Ordering::SeqCst);

                        set_log_level(log_level);

                        let queues = open_queues(&run_config_bundle)?;
                        let working_directory_pool = open_working_directory_pool(
                            &run_config_bundle.run_config,
                            &working_directory_base_dir,
                        )?;
                        run_queues(
                            run_config_bundle,
                            queues,
                            working_directory_base_dir,
                            working_directory_pool,
                            false,
                            Some(daemon_check_exit.clone()),
                        )?;
                        Ok(())
                    };

                    let other_restart_checks = restart_for_executable_change_opts
                        .to_restarter(
                            DEFAULT_RESTART_ON_UPGRADES,
                            TimestampFormatter {
                                use_rfc3339: true,
                                local_time,
                            },
                        )?
                        .and_config_change_opts(
                            restart_for_config_change_opts,
                            DEFAULT_RESTART_ON_CONFIG_CHANGE,
                            config_file,
                        );

                    let daemon = Daemon {
                        opts,
                        restart_on_failures_default: true,
                        restart_opts: None,
                        timestamp_opts: TimestampOpts {
                            use_rfc3339: true,
                            mode: TimestampMode::Automatic {
                                mark_added_timestamps: true,
                            },
                        },
                        paths,
                        other_restart_checks,
                        run,
                    };

                    let r = daemon.execute(action, DEFAULT_IS_HARD)?;
                    Ok(Some(r))
                }
            }
        }

        SubCommand::Wd { subcommand } => {
            let mut working_directory_pool =
                open_working_directory_pool(conf, working_directory_base_dir.clone(), true)?
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
                    .lock_mut("evobench-jobs SubCommand::Wd do_mark")
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
                    if let Some(working_directory_change_signals) = working_directory_change_signals
                    {
                        working_directory_change_signals.send_signal();
                    }
                    Ok(Some(Marked::OldStatus(original_status)))
                } else {
                    Ok(None)
                }
            };

            let mut working_directory_change_signals =
                lazyresult!(open_working_directory_change_signals(conf));

            match subcommand {
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
                        let mut all_entries: Vec<_> =
                            working_directory_pool.all_entries().collect();
                        if numeric_sort {
                            // Leave as is, it's already sorted
                        } else {
                            all_entries.sort_by(|a, b| a.1.last_use.cmp(&b.1.last_use))
                        }
                        all_entries.iter().map(|(id, _)| *id).collect()
                    };

                    for id in all_ids {
                        let mut lock = working_directory_pool.lock_mut("evobench-jobs Wd::List")?;
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
                    Ok(None)
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
                        let mut lock =
                            working_directory_pool.lock_mut("evobench-jobs Wd::Cleanup")?;
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
                    Ok(None)
                }
                Wd::Delete {
                    dry_run,
                    force,
                    verbose,
                    ids,
                } => {
                    let mut lock_mut =
                        working_directory_pool.lock_mut("evobench-jobs Wd::Delete")?;
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
                    Ok(None)
                }
                Wd::Log { list, id } => {
                    let working_directory_path =
                        if let Some(wd) = working_directory_pool.get_working_directory(id) {
                            wd.working_directory_path()
                        } else {
                            let mut out = BufWriter::new(stderr().lock());
                            writeln!(
                                &mut out,
                                "NOTE: working directory with id {id} does not exist. \
                                 Looking for log files anyway."
                            )?;
                            out.flush()?;
                            if !list {
                                thread::sleep(Duration::from_millis(1400));
                            }
                            working_directory_pool.get_working_directory_path(id)
                        };

                    if list {
                        let mut out = BufWriter::new(stdout().lock());
                        for (standard_log_path, _run_id) in
                            working_directory_path.standard_log_paths()?
                        {
                            out.write_all(standard_log_path.as_os_str().as_bytes())?;
                            out.write_all(b"\n")?;
                        }
                        out.flush()?;
                    } else {
                        let (standard_log_path, _run_id) = working_directory_path
                            .last_standard_log_path()?
                            .ok_or_else(|| {
                                anyhow!("could not find a log file for working directory {id}")
                            })?;

                        let pager = match std::env::var("PAGER") {
                            Ok(s) => s,
                            Err(e) => match e {
                                std::env::VarError::NotPresent => "less".into(),
                                _ => bail!("can't decode PAGER env var: {e:#}"),
                            },
                        };

                        let mut cmd = Command::new(&pager);
                        cmd.arg(standard_log_path);
                        return Err(cmd.exec())
                            .with_context(|| anyhow!("executing pager {pager:?}"));
                    }
                    Ok(None)
                }
                Wd::Mark { ids } => {
                    for id in ids {
                        if do_mark(Status::Examination, true, id, None)?.is_none() {
                            warn!("there is no working directory for id {id}");
                        }
                    }
                    Ok(None)
                }
                Wd::Unmark { ids } => {
                    for id in ids {
                        if do_mark(Status::Error, true, id, None)?.is_none() {
                            warn!("there is no working directory for id {id}");
                        }
                    }
                    Ok(None)
                }
                Wd::Recycle { ids } => {
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
                    Ok(None)
                }
                Wd::Enter {
                    mark,
                    unmark,
                    force,
                    no_fetch,
                    id,
                } => {
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
                        .map(|(k, v)| {
                            bash_export_variable_string(k, &v.to_string_lossy(), "  ", "\n")
                        })
                        .join("");

                    let shell = match std::env::var("SHELL") {
                        Ok(s) => s,
                        Err(e) => match e {
                            env::VarError::NotPresent => "bash".into(),
                            env::VarError::NotUnicode(os_string) => bail!(
                                "the SHELL environment variable is not in unicode: {os_string:?}"
                            ),
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
                                        .lock_mut("evobench-jobs Wd::Enter do_revert")?
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
}

fn main() -> Result<()> {
    if let Some(execution_result) = run()? {
        execution_result.daemon_cleanup();
    }
    Ok(())
}
