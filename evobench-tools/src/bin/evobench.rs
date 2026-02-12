use anyhow::{Context, Result, anyhow, bail};
use auri::url_encoding::url_decode;
use chj_unix_util::{
    daemon::{
        Daemon, DaemonCheckExit, DaemonMode, DaemonOpts, DaemonPaths, ExecutionResult,
        warrants_restart::{
            RestartForConfigChangeOpts, RestartForExecutableChangeOpts,
            RestartForExecutableOrConfigChange,
        },
    },
    logging::{TimestampMode, TimestampOpts},
    polling_signals::PollingSignalsSender,
    timestamp_formatter::TimestampFormatter,
};
use clap::{CommandFactory, Parser};
use itertools::Itertools;
use url::Url;

use std::{
    io::{StdoutLock, Write, stdout},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, exit},
    str::FromStr,
    sync::{Arc, atomic::Ordering},
    thread,
    time::Duration,
};

use evobench_tools::{
    clap_styles::clap_styles,
    config_file::{self, ConfigFile, save_config_file},
    ctx, debug,
    get_terminal_width::get_terminal_width,
    git::GitHash,
    info,
    io_utils::shell::preferred_shell,
    lazyresult,
    run::{
        bench_tmp_dir::bench_tmp_dir,
        benchmarking_job::{BenchmarkingJobOpts, BenchmarkingJobReasonOpt},
        config::{RunConfig, RunConfigBundle, RunConfigOpts},
        global_app_state_dir::GlobalAppStateDir,
        insert_jobs::{DryRunOpt, ForceOpt, QuietOpt, insert_jobs},
        open_run_queues::open_run_queues,
        output_directory::structure::OutputSubdir,
        run_context::RunContext,
        run_job::JobRunner,
        run_queues::RunQueues,
        sub_command::{
            insert::{Insert, InsertBenchmarkingJobOpts, InsertOpts},
            list::ListOpts,
            list_all::ListAllOpts,
            open_polling_pool, open_working_directory_pool,
            wd::{
                Wd, get_run_lock, open_queue_change_signals, open_working_directory_change_signals,
            },
        },
        versioned_dataset_dir::VersionedDatasetDir,
        working_directory_pool::{WorkingDirectoryPool, WorkingDirectoryPoolBaseDir},
    },
    serde_types::date_and_time::{DateTimeWithOffset, LOCAL_TIME},
    utillib::{
        arc::CloneArc,
        into_arc_path::IntoArcPath,
        logging::{LogLevel, LogLevelOpts, set_log_level},
    },
};

type CheckExit<'t> =
    DaemonCheckExit<'t, RestartForExecutableOrConfigChange<Arc<ConfigFile<RunConfigOpts>>>>;

const DEFAULT_RESTART_ON_UPGRADES: bool = true;
const DEFAULT_RESTART_ON_CONFIG_CHANGE: bool = true;

/// True since the configuration uses times, too, and those are
/// probably better local time, and in general, just use whatever the
/// TZ is set to. You can set TZ to UTC, too.
const LOCAL_TIME_DEFAULT: bool = true;

#[derive(clap::Parser, Debug)]
#[command(
    next_line_help = true,
    styles = clap_styles(),
    term_width = get_terminal_width(4),
    allow_hyphen_values = true,
    bin_name = "evobench",
)]
/// Schedule and query benchmarking jobs.
struct Opts {
    #[clap(flatten)]
    log_level_opts: LogLevelOpts,

    /// Alternative to --quiet / --verbose / --debug for setting the
    /// log-level (an error is reported if both are given and they
    /// don't agree)
    #[clap(long)]
    log_level: Option<LogLevel>,

    /// Override the path to the config file (default: the paths
    /// `~/.evobench.*` where a single one exists where the `*` is the
    /// suffix for one of the supported config file formats (run
    /// `config-formats` to get the list).
    #[clap(long)]
    config: Option<PathBuf>,

    /// The subcommand to run. Use `--help` after the sub-command to
    /// get a list of the allowed options there.
    #[clap(subcommand)]
    subcommand: SubCommand,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// Show the table of all inserted jobs, including already
    /// processed ones. This is the table that `evobench insert`
    /// checks to avoid duplicate inserts by default. .
    ListAll {
        #[clap(flatten)]
        opts: ListAllOpts,
    },

    /// List the jobs that are being processed (per queue)
    List {
        #[clap(flatten)]
        opts: ListOpts,
    },

    /// Insert zero or more jobs, either from a complete benchmarking
    /// job description file, or using the job templates from one list
    /// from the configuration combined with zero or more commits. For
    /// automatic periodic insertion, see the `poll` sub-command
    /// instead. .
    Insert {
        #[clap(flatten)]
        opts: InsertOpts,

        #[clap(subcommand)]
        method: Insert,
    },

    /// Insert jobs for new commits on branch names configured in the
    /// config option `remote_branch_names_for_poll`. For one-off
    /// manual insertion see `insert` instead. .
    Poll {
        // No QuietOpt since that must be the default. Also, another
        // force option since the help text is different here.
        /// Normally, the same job parameters are only inserted once,
        /// subsequent polls yielding the same commits remain
        /// no-ops. This overrides the check and inserts the found
        /// commits anyway. .
        #[clap(long)]
        force: bool,

        /// Suppress printing the "inserted n jobs" message when n >
        /// 0, i.e. always be quiet.
        #[clap(long)]
        quiet: bool,

        /// Report an error if any of the given (branch or other)
        /// names do not resolve.
        #[clap(long)]
        fail: bool,

        #[clap(flatten)]
        dry_run_opt: DryRunOpt,

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

    /// Parse URLs to output directories
    Url {
        /// Open a new $SHELL (or bash) in the output
        /// directory. Default: print the path instead. .
        #[clap(long)]
        cd: bool,

        /// The URL or (partial) path to parse
        url: String,
    },

    /// General program status information (but also see `list`, `wd
    /// list`, `list-all`, `run daemon status`, `poll daemon status`)
    Status {},

    /// Generate a shell completions file. (Redirect stdout to a file
    /// that is `source`d from the shell.)
    Completions {
        /// The shell to generate the completions for.
        #[arg(value_enum)]
        shell: clap_complete_command::Shell,
    },

    /// Show the supported config format types.
    ConfigFormats,

    /// Re-encode the config file (with the serialization type
    /// determined by the file extension) and save it at the given
    /// path.
    ConfigSave { output_path: PathBuf },
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum RunMode {
    /// Carry out a single run
    One {
        /// Exit with code 1 if there is no runnable job / there were
        /// no jobs to insert.
        #[clap(long)]
        false_if_none: bool,
    },
    /// Run forever, until terminated (note: evobench uses
    /// restart-on-failures and local-time by default; the local-time
    /// setting has no effect on times in the config file, those are
    /// always parsed as local-time)
    Daemon {
        #[clap(flatten)]
        opts: DaemonOpts,
        #[clap(flatten)]
        restart_for_executable_change_opts: RestartForExecutableChangeOpts,
        #[clap(flatten)]
        restart_for_config_change_opts: RestartForConfigChangeOpts,

        /// The logging level while running as daemon (overrides the
        /// top-level logging options like --verbose, --debug,
        /// --quiet) (default: "info" for run daemon, "warn" for poll
        /// daemon)
        #[clap(short, long)]
        log_level: Option<LogLevel>,

        /// Whether to run in the foreground, or start or stop a
        /// daemon running in the background (or report the status
        /// about it). Give `help` to see the options. evobench
        /// defaults to the 'hard' actions.
        action: DaemonMode,
    },
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
    queue_change_signals: PollingSignalsSender,
) -> Result<RunResult> {
    let conf = &run_config_bundle.shareable.run_config;
    let _run_lock = get_run_lock(conf)?;

    let mut run_context = RunContext::default();
    let versioned_dataset_dir = VersionedDatasetDir::new();

    // Test-run
    if let Some(versioned_dataset_base_dir) = &conf.versioned_datasets_base_dir {
        debug!("Test-running versioned dataset search");

        let working_directory_id;
        {
            let mut pool = working_directory_pool.lock_mut("evobench::run_queues")?;
            working_directory_id = pool.get_first()?;
            pool.clear_current_working_directory()?;
        }
        debug!("Got working directory {working_directory_id:?}");
        let ((), token) = working_directory_pool.process_in_working_directory(
            working_directory_id,
            &DateTimeWithOffset::now(None),
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
        ).context("while early-checking versioned datasets at startup")?;
        working_directory_pool.working_directory_cleanup(token)?;
    }

    let mut working_directory_change_signals = open_working_directory_change_signals(conf)?;

    loop {
        // XX handle errors without exiting? Or do that above

        let queues_data = queues.data()?;

        let ran = queues_data.run_next_job(
            JobRunner {
                working_directory_pool: &mut working_directory_pool,
                output_base_dir: &conf.output_dir.path,
                timestamp: DateTimeWithOffset::now(None),
                shareable_config: &run_config_bundle.shareable,
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
        if working_directory_change_signals.got_signals() {
            info!("the working directory pool was updated outside the app, reload it");
            working_directory_pool = open_working_directory_pool(
                conf,
                working_directory_base_dir.clone_arc(),
                false,
                Some(queue_change_signals.clone()),
            )?
            .into_inner();
        }
    }
}

struct EvobenchDaemon<F: FnOnce(CheckExit) -> Result<()>> {
    paths: DaemonPaths,
    opts: DaemonOpts,
    log_level: LogLevel,
    restart_for_executable_change_opts: RestartForExecutableChangeOpts,
    restart_for_config_change_opts: RestartForConfigChangeOpts,
    config_file: Arc<ConfigFile<RunConfigOpts>>,
    inner_run: F,
}

impl<F: FnOnce(CheckExit) -> Result<()>> EvobenchDaemon<F> {
    fn into_daemon(
        self,
    ) -> Result<
        Daemon<
            RestartForExecutableOrConfigChange<Arc<ConfigFile<RunConfigOpts>>>,
            impl FnOnce(CheckExit) -> Result<()>,
        >,
    > {
        let Self {
            log_level,
            restart_for_executable_change_opts,
            restart_for_config_change_opts,
            opts,
            paths,
            config_file,
            inner_run,
        } = self;
        let local_time = opts.logging_opts.local_time(LOCAL_TIME_DEFAULT);

        let run = move |daemon_check_exit: CheckExit| -> Result<()> {
            // Use the requested time setting for
            // local time stamp generation, too (now
            // the default is UTC, which is expected
            // for a daemon).
            LOCAL_TIME.store(local_time, Ordering::SeqCst);

            set_log_level(log_level);

            inner_run(daemon_check_exit)
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

        Ok(Daemon {
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
            local_time_default: LOCAL_TIME_DEFAULT,
        })
    }
}

const DEFAULT_IS_HARD: bool = true;

fn run() -> Result<Option<ExecutionResult>> {
    let Opts {
        log_level_opts,
        log_level,
        config,
        subcommand,
    } = Opts::parse();

    let log_level = log_level_opts.xor_log_level(log_level)?;

    set_log_level(log_level);
    // Interactive use should get local time. (Daemon mode possibly
    // overwrites this.) true or LOCAL_TIME_DEFAULT?
    LOCAL_TIME.store(LOCAL_TIME_DEFAULT, Ordering::SeqCst);

    let config: Option<Arc<Path>> = config.map(Into::into);

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

    let conf = &run_config_bundle.shareable.run_config;

    let working_directory_base_dir = Arc::new(WorkingDirectoryPoolBaseDir::new(
        conf.working_directory_pool.base_dir.clone(),
        &|| {
            run_config_bundle
                .shareable
                .global_app_state_dir
                .working_directory_pool_base()
        },
    )?);

    let queues = lazyresult! {
        open_run_queues(&run_config_bundle.shareable)
    };
    // Do not attempt to pass those to `open_run_queues` above; maybe
    // they are needed without needing the queues?
    let queue_change_signals = {
        let gasd = run_config_bundle.shareable.global_app_state_dir.clone();
        lazyresult!(move open_queue_change_signals(&gasd).map(|s| s.sender()))
    };

    match subcommand {
        SubCommand::ConfigFormats => unreachable!("already dispatched above"),

        SubCommand::ConfigSave { output_path } => {
            save_config_file(&output_path, &**run_config_bundle.config_file)?;
            Ok(None)
        }

        SubCommand::ListAll { opts } => {
            opts.run(&run_config_bundle.shareable)?;
            Ok(None)
        }

        SubCommand::List { opts } => {
            let (queues, regenerate_index_files) = queues.force()?;
            opts.run(conf, &working_directory_base_dir, queues)?;
            regenerate_index_files.run_one();
            Ok(None)
        }

        SubCommand::Insert { opts, method } => {
            let (queues, regenerate_index_files) = queues.force()?;
            let n = method.run(opts, &run_config_bundle, &queues)?;
            println!("Inserted {n} jobs.");
            regenerate_index_files.run_one();
            Ok(None)
        }

        SubCommand::Poll {
            force,
            quiet,
            fail,
            dry_run_opt,
            mode,
        } => {
            // Returns whether at least 1 job was inserted
            let try_run_poll = |daemon_check_exit: Option<CheckExit>| -> Result<bool> {
                loop {
                    let (commits, non_resolving) = {
                        let mut polling_pool = open_polling_pool(&run_config_bundle.shareable)?;

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
                            insert_benchmarking_job_opts: InsertBenchmarkingJobOpts {
                                reason: BenchmarkingJobReasonOpt {
                                    reason: branch_name.as_str().to_owned().into(),
                                },
                                benchmarking_job_settings: (*conf.benchmarking_job_settings)
                                    .clone(),
                                priority: None,
                                initial_boost: None,
                            },
                            commit_id,
                        };
                        benchmarking_jobs.append(&mut opts.complete_jobs(&job_templates));
                    }

                    let n_original = benchmarking_jobs.len();
                    let (queues, regenerate_index_files) = queues.force()?;
                    let n = insert_jobs(
                        benchmarking_jobs,
                        &run_config_bundle.shareable,
                        dry_run_opt.clone(),
                        ForceOpt { force },
                        // Must use quiet so that it can try to insert *all*
                        // given jobs (XX: should it continue even with
                        // errors, for the other code places?)
                        QuietOpt { quiet: true },
                        &queues,
                    )?;
                    regenerate_index_files.run_one();

                    if non_resolving.is_empty() || !fail {
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
                    let paths = conf.polling_daemon.clone();
                    let config_file = run_config_bundle.config_file.clone_arc();
                    let inner_run = |daemon_check_exit: CheckExit| -> Result<()> {
                        try_run_poll(Some(daemon_check_exit))?;
                        Ok(())
                    };
                    let daemon = EvobenchDaemon {
                        paths,
                        opts,
                        log_level: log_level.unwrap_or(LogLevel::Warn),
                        restart_for_executable_change_opts,
                        restart_for_config_change_opts,
                        config_file,
                        inner_run,
                    }
                    .into_daemon()?;
                    let r = daemon.execute(action, DEFAULT_IS_HARD)?;
                    Ok(Some(r))
                }
            }
        }

        SubCommand::Run { mode } => {
            let open_working_directory_pool = |conf: &RunConfig| -> Result<_> {
                Ok(open_working_directory_pool(
                    conf,
                    working_directory_base_dir.clone(),
                    false,
                    Some(queue_change_signals.force()?.clone()),
                )?
                .into_inner())
            };

            match mode {
                RunMode::One { false_if_none } => {
                    let (queues, regenerate_index_files) = queues.into_value()?;
                    let working_directory_pool = open_working_directory_pool(conf)?;
                    let r = run_queues(
                        run_config_bundle,
                        queues,
                        working_directory_base_dir,
                        working_directory_pool,
                        true,
                        None,
                        queue_change_signals.force()?.clone(),
                    );
                    regenerate_index_files.run_one();
                    match r? {
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
                    let config_file = run_config_bundle.config_file.clone_arc();
                    let (queues, regenerate_index_files) = queues.into_value()?;

                    // The code that runs in the daemon and executes the jobs
                    let inner_run = |daemon_check_exit: CheckExit| -> Result<()> {
                        regenerate_index_files.spawn_runner_thread()?;

                        let conf = &run_config_bundle.shareable.run_config;
                        let working_directory_pool = open_working_directory_pool(conf)?;
                        run_queues(
                            run_config_bundle,
                            queues,
                            working_directory_base_dir.clone(),
                            working_directory_pool,
                            false,
                            Some(daemon_check_exit.clone()),
                            queue_change_signals.force()?.clone(),
                        )?;
                        Ok(())
                    };
                    let daemon = EvobenchDaemon {
                        paths,
                        opts,
                        log_level: log_level.unwrap_or(LogLevel::Info),
                        restart_for_executable_change_opts,
                        restart_for_config_change_opts,
                        config_file,
                        inner_run,
                    }
                    .into_daemon()?;
                    let r = daemon.execute(action, DEFAULT_IS_HARD)?;
                    Ok(Some(r))
                }
            }
        }

        SubCommand::Wd { subcommand } => {
            subcommand.run(
                &run_config_bundle.shareable,
                &working_directory_base_dir,
                queue_change_signals.force()?.clone(),
            )?;
            Ok(None)
        }

        SubCommand::Url { cd, url } => {
            // Allow both copy-paste from web browser (at least
            // Firefox encodes '=' in path part via url_encoding), and
            // local paths. First see if it's a URL (XX false
            // positives?).
            let path = match Url::from_str(&url) {
                Ok(mut url) => {
                    url.set_fragment(None);
                    url.set_query(None);
                    // At that point '=' are still URL-encoded, thus:
                    url_decode(url.as_str())?
                }
                Err(_) => {
                    // Unparseable as Url; no problem, just can't
                    // remove any fragment and query parts. Don't just
                    // url_decode, since custom variables might
                    // contain such parts, too; only do it if there
                    // are no '=' in the path, OK?
                    if url.contains('=') {
                        url
                    } else {
                        url_decode(&url)?
                    }
                }
            }
            .into_arc_path();

            let subdir = OutputSubdir::try_from(path)?;
            let subdir = subdir.replace_base_path(conf.output_dir.path.clone_arc());
            let local_path = subdir.to_path();

            if cd {
                let shell = preferred_shell()?;
                Err(Command::new(&shell).current_dir(&local_path).exec())
                    .map_err(ctx!("executing {shell:?} in {local_path:?}"))?;
            } else {
                (|| -> Result<(), std::io::Error> {
                    let mut out = stdout().lock();
                    out.write_all(local_path.as_os_str().as_bytes())?;
                    out.write_all(b"\n")?;
                    out.flush()
                })()
                .map_err(ctx!("stdout"))?;
            }

            Ok(None)
        }

        SubCommand::Status {} => {
            let show_status =
                |daemon_name: &str, paths: &DaemonPaths, out: &mut StdoutLock| -> Result<_> {
                    let daemon = EvobenchDaemon {
                        paths: paths.clone(),
                        opts: DaemonOpts::default(),
                        log_level: LogLevel::Quiet,
                        restart_for_executable_change_opts: RestartForExecutableChangeOpts::default(
                        ),
                        restart_for_config_change_opts: RestartForConfigChangeOpts::default(),
                        config_file: run_config_bundle.config_file.clone_arc(),
                        inner_run: |_| Ok(()),
                    }
                    .into_daemon()?;
                    let s = daemon.status_string(true)?;
                    let logs = &paths.log_dir;
                    writeln!(out, "  {daemon_name} daemon: {s}, logs: {logs:?}")?;
                    Ok(())
                };

            let mut out = stdout().lock();
            writeln!(
                &mut out,
                "Evobench system status and configuration information:\n"
            )?;
            show_status(" run", &conf.run_jobs_daemon, &mut out)?;
            show_status("poll", &conf.polling_daemon, &mut out)?;

            // writeln!(&mut out, "\nPaths:")?;
            writeln!(&mut out, "")?;
            writeln!(
                &mut out,
                "               Queues: {:?}",
                conf.queues
                    .run_queues_basedir(false, &run_config_bundle.shareable.global_app_state_dir)?
            )?;
            writeln!(
                &mut out,
                "  Working directories: {:?} -- but modify via `evobench wd` only",
                working_directory_base_dir.path()
            )?;
            writeln!(
                &mut out,
                "        Temporary dir: {:?}",
                bench_tmp_dir()?.as_ref(),
            )?;
            writeln!(
                &mut out,
                "              Outputs: {:?}",
                conf.output_dir.path,
            )?;
            writeln!(&mut out, "          Outputs URL: {:?}", conf.output_dir.url,)?;
            writeln!(
                &mut out,
                "          Config file: {:?}",
                run_config_bundle.config_file.path()
            )?;

            out.flush()?;
            Ok(None)
        }

        SubCommand::Completions { shell } => {
            shell.generate(&mut Opts::command(), &mut std::io::stdout());
            Ok(None)
        }
    }
}

fn main() -> Result<()> {
    if let Some(execution_result) = run()? {
        execution_result.daemon_cleanup();
    }
    Ok(())
}
