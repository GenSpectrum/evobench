use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt::{Debug, Display},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local};
use cj_path_util::path_util::AppendToPath;
use kstring::KString;

use crate::{
    config_file::{ConfigFile, DefaultConfigPath, ron_to_string_pretty},
    date_and_time::time_ranges::{DateTimeRange, LocalNaiveTimeRange},
    info,
    io_utils::{bash::bash_string_from_cmd, div::create_dir_if_not_exists},
    key::CustomParameters,
    run::env_vars::AllowableCustomEnvVar,
    serde::{
        allowed_env_var::AllowedEnvVar,
        date_and_time::LocalNaiveTime,
        git_branch_name::GitBranchName,
        git_url::GitUrl,
        priority::Priority,
        proper_dirname::ProperDirname,
        proper_filename::ProperFilename,
        regex::SerializableRegex,
        tilde_path::TildePath,
        val_or_ref::{ValOrRef, ValOrRefTarget},
    },
    util::grep_diff::LogExtract,
    utillib::arc::CloneArc,
};

use super::{
    benchmarking_job::BenchmarkingJobSettingsOpts, custom_parameter::AllowedCustomParameter,
    global_app_state_dir::GlobalAppStateDir, working_directory_pool::WorkingDirectoryPoolOpts,
};

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ScheduleCondition {
    /// Run jobs in this queue once right away
    Immediately {
        /// A description of the situation during which jobs in this
        /// queue are executed; all jobs of the same context (and same
        /// key) are grouped together and evaluated to "summary-" file
        /// names with this string appended. Meant to reflect
        /// conditions that might influence the results;
        /// e.g. "immediate" or "night".
        situation: ProperFilename,
    },

    /// Run jobs in this queue between the given times on every day
    /// (except when one of the times is not valid or ambiguous on a
    /// given day due to DST changes). Jobs started before the end of
    /// the window are finished, though.
    LocalNaiveTimeWindow {
        /// The priority of this queue--it is added to the priority of
        /// jobs in this queue. By default, 1.5 is used.
        priority: Option<Priority>,
        /// A description of the situation during which jobs in this
        /// queue are executed; all jobs of the same context (and same
        /// key) are grouped together and evaluated to "summary-" file
        /// names with this string appended. Meant to reflect
        /// conditions that might influence the results;
        /// e.g. "immediate" or "night".
        situation: ProperFilename,
        /// A command and arguments, run with "stop" at the `from`
        /// time and with "start" when done / at the `to` time.
        stop_start: Option<Vec<String>>,
        /// If true, run the `BenchmarkingJob`s in this queue until
        /// their own `count` reaches zero or the time window runs out
        /// (each job is rescheduled to the end of the queue after a
        /// run, meaning the jobs are alternating). If false, each job
        /// is run once and then moved to the next queue.
        repeatedly: bool,
        /// If true, when time runs out, move all remaining jobs to
        /// the next queue; if false, the jobs remain and are
        /// scheduled again in the same time window on the next day.
        move_when_time_window_ends: bool,
        /// Times in the time zone that the daemon is running with (to
        /// change that, set `TZ` env var to area/city, or the default
        /// time zone via dpkg-reconfigure).
        from: LocalNaiveTime,
        to: LocalNaiveTime,
    },

    /// A queue that is never run and never emptied, to add to the end
    /// of the queue pipeline to take up jobs that have been expelled
    /// from the second last queue, for informational purposes.
    Inactive,
}

impl Display for ScheduleCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleCondition::Immediately { situation } => {
                write!(f, "Immediately {:?}", situation.as_str())
            }
            ScheduleCondition::LocalNaiveTimeWindow {
                priority: _,
                situation,
                stop_start,
                repeatedly,
                move_when_time_window_ends,
                from,
                to,
            } => {
                let rep = if *repeatedly { "repeatedly" } else { "once" };
                let mov = if *move_when_time_window_ends {
                    "move"
                } else {
                    "stay"
                };
                let cmd = if let Some(st) = stop_start {
                    bash_string_from_cmd(st)
                } else {
                    "-".into()
                };
                let pri: f64 = self
                    .priority()
                    .expect("LocalNaiveTimeWindow *does* have priority field")
                    .into();
                write!(
                    f,
                    "LocalNaiveTimeWindow {:?} {from} - {to} pri={pri}: {rep}, {mov}, \"{cmd}\"",
                    situation.as_str()
                )
            }
            ScheduleCondition::Inactive => f.write_str("Inactive"),
        }
    }
}

impl ScheduleCondition {
    pub const TIMED_QUEUE_DEFAULT_PRIORITY: Priority = Priority::new_unchecked(1.5);

    /// Whether this queue will never run its jobs
    pub fn is_inactive(&self) -> bool {
        match self {
            ScheduleCondition::Inactive => true,
            _ => false,
        }
    }

    pub fn time_range(&self) -> Option<(LocalNaiveTime, LocalNaiveTime)> {
        match self {
            ScheduleCondition::Immediately { situation: _ } => None,
            ScheduleCondition::LocalNaiveTimeWindow {
                priority: _,
                situation: _,
                stop_start: _,
                repeatedly: _,
                move_when_time_window_ends: _,
                from,
                to,
            } => Some((from.clone(), to.clone())),
            ScheduleCondition::Inactive => None,
        }
    }

    pub fn stop_start(&self) -> Option<&[String]> {
        match self {
            ScheduleCondition::Immediately { situation: _ } => None,
            ScheduleCondition::LocalNaiveTimeWindow {
                priority: _,
                situation: _,
                stop_start,
                repeatedly: _,
                move_when_time_window_ends: _,
                from: _,
                to: _,
            } => stop_start.as_deref(),
            ScheduleCondition::Inactive => None,
        }
    }

    /// Returns true if the condition offers that flag *and* it is true
    pub fn move_when_time_window_ends(&self) -> bool {
        match self {
            ScheduleCondition::Immediately { situation: _ } => false,
            ScheduleCondition::LocalNaiveTimeWindow {
                priority: _,
                situation: _,
                stop_start: _,
                repeatedly: _,
                move_when_time_window_ends,
                from: _,
                to: _,
            } => *move_when_time_window_ends,
            ScheduleCondition::Inactive => false,
        }
    }

    pub fn situation(&self) -> Option<&ProperFilename> {
        match self {
            ScheduleCondition::Immediately { situation } => Some(situation),
            ScheduleCondition::LocalNaiveTimeWindow {
                priority: _,
                situation,
                stop_start: _,
                repeatedly: _,
                move_when_time_window_ends: _,
                from: _,
                to: _,
            } => Some(situation),
            ScheduleCondition::Inactive => None,
        }
    }

    pub fn priority(&self) -> Option<Priority> {
        match self {
            ScheduleCondition::Immediately { situation: _ } => Some(Priority::default()),
            ScheduleCondition::LocalNaiveTimeWindow {
                priority,
                situation: _,
                stop_start: _,
                repeatedly: _,
                move_when_time_window_ends: _,
                from: _,
                to: _,
            } => Some(priority.unwrap_or(Self::TIMED_QUEUE_DEFAULT_PRIORITY)),
            ScheduleCondition::Inactive => None,
        }
    }

    /// Returns an optional time window (given if runnable due to
    /// being in this time window) if runnable
    pub fn is_runnable_at(
        &self,
        reference_time: DateTime<Local>,
    ) -> Option<Option<DateTimeRange<Local>>> {
        match self {
            ScheduleCondition::Immediately { situation: _ } => Some(None),
            ScheduleCondition::LocalNaiveTimeWindow {
                priority: _,
                situation: _,
                stop_start: _,
                repeatedly: _,
                move_when_time_window_ends: _,
                from,
                to,
            } => {
                let ltr = LocalNaiveTimeRange {
                    from: *from,
                    to: *to,
                };
                let dtr: Option<DateTimeRange<Local>> = ltr.after_datetime(&reference_time, true);
                if let Some(dtr) = dtr {
                    if dtr.contains(&reference_time) {
                        Some(Some(dtr))
                    } else {
                        None
                    }
                } else {
                    info!("times in {ltr} do not resolve for {reference_time}");
                    None
                }
            }
            ScheduleCondition::Inactive => None,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuesConfig {
    /// If not given, `~/.evobench-jobs/queues/` is used. Also used for
    /// locking the `run` action of evobench-jobs, to ensure only one
    /// benchmarking job is executed at the same time--if you
    /// configure multiple such directories then you don't have this
    /// guarantee any more. Supports `~/`
    /// for specifying the home directory.
    pub run_queues_basedir: Option<TildePath<PathBuf>>,

    /// The queues to use (file names, without '/'), and their
    /// scheduled execution condition
    pub pipeline: Vec<(ProperFilename, ScheduleCondition)>,

    /// The queue where to put jobs when they run out of
    /// `error_budget` (if `None` is given, the jobs will be dropped--
    /// silently unless verbose flag is given). Should be of
    /// scheduling type Inactive (or perhaps a future messaging
    /// queue).
    pub erroneous_jobs_queue: Option<(ProperFilename, ScheduleCondition)>,

    /// The queue where to put jobs when they are finished
    /// successfully (if `None` is given, the jobs will be dropped--
    /// silently unless verbose flag is given).
    pub done_jobs_queue: Option<(ProperFilename, ScheduleCondition)>,

    /// How many jobs to show in the extra queues
    /// (`erroneous_jobs_queue` and `done_jobs_queue`) when no `--all`
    /// option is given
    pub view_jobs_max_len: usize,
}

impl QueuesConfig {
    pub fn run_queues_basedir(
        &self,
        create_if_not_exists: bool,
        global_app_state_dir: &GlobalAppStateDir,
    ) -> Result<PathBuf> {
        if let Some(base_dir) = &self.run_queues_basedir {
            let base_dir = base_dir.resolve()?;
            if create_if_not_exists {
                create_dir_if_not_exists(&base_dir, "queues base directory")?;
            }
            Ok(base_dir)
        } else {
            global_app_state_dir.run_queues_basedir()
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[serde(rename = "RemoteRepository")]
pub struct RemoteRepositoryOpts {
    /// The Git repository to clone the target project from
    pub url: GitUrl,

    /// The remote branches to track
    pub remote_branch_names_for_poll:
        BTreeMap<GitBranchName, ValOrRef<JobTemplateListsField, Vec<JobTemplateOpts>>>,
}

pub struct RemoteRepository {
    pub url: GitUrl,
    pub remote_branch_names_for_poll: BTreeMap<GitBranchName, Arc<[JobTemplate]>>,
}

impl RemoteRepositoryOpts {
    fn check(
        &self,
        job_template_lists: &BTreeMap<KString, Arc<[JobTemplate]>>,
        targets: &BTreeMap<ProperDirname, Arc<BenchmarkingTarget>>,
    ) -> Result<RemoteRepository> {
        let Self {
            url,
            remote_branch_names_for_poll,
        } = self;

        let remote_branch_names_for_poll = remote_branch_names_for_poll
            .iter()
            .map(|(branch_name, job_template_optss)| -> Result<_> {
                let job_templates: ValOrRef<JobTemplateListsField, Arc<[JobTemplate]>> =
                    job_template_optss.try_map(
                        |job_template_optss: &Vec<JobTemplateOpts>| -> Result<Arc<[JobTemplate]>> {
                            job_template_optss
                                .iter()
                                .map(|job_template_opts| job_template_opts.check(targets))
                                .collect()
                        },
                    )?;
                let job_templates = job_templates.value_with_backing(job_template_lists)?;
                Ok((branch_name.clone(), job_templates.clone_arc()))
            })
            .collect::<Result<_>>()?;

        Ok(RemoteRepository {
            url: url.clone(),
            remote_branch_names_for_poll,
        })
    }
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// What command to run on the target project to execute a
/// benchmarking run; the env variables configured in CustomParameters
/// are set when running this command.
pub struct BenchmarkingCommand {
    /// The name is matched on the `target_name` field in
    /// `JobTemplate`, and it is used as the first path segment below
    /// `output_base_dir` for storing the results. It will also be
    /// shown by `evobench-jobs list`.
    pub target_name: ProperDirname,

    /// Relative path to the subdirectory (provide "." for the top
    /// level of the working directory) where to run the command
    pub subdir: PathBuf,

    /// Name or path to the command to run, e.g. "make"
    pub command: PathBuf,

    /// Arguments to the command, e.g. "bench"
    pub arguments: Vec<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkingTarget {
    pub benchmarking_command: Arc<BenchmarkingCommand>,

    /// Which custom environment variables are allowed, required, and
    /// of what type (format) they must be.
    pub allowed_custom_parameters:
        BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, AllowedCustomParameter>,

    /// Optional list of `LogExtract` declarations, to extract time
    /// spans from the stdout/stderr of the benchmark run. (Note: this
    /// is not and does not include the file optionally written by the
    /// target application to the path in the `BENCH_OUTPUT_LOG` env
    /// var!--Possible todo: offer something separate for that file?)
    pub log_extracts: Option<Vec<LogExtract>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename = "JobTemplate")]
pub struct JobTemplateOpts {
    priority: Priority,
    initial_boost: Priority,
    target_name: ProperDirname,
    // Using `String` for values--type checking is done in conversion
    // to `JobTemplate` (don't want to use another enum here that
    // would be required, and `allowed_custom_parameters` already have
    // the type, no *need* to specify it again, OK?)
    custom_parameters: BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, KString>,
}

pub struct JobTemplate {
    pub priority: Priority,
    pub initial_boost: Priority,
    pub command: Arc<BenchmarkingCommand>,
    pub custom_parameters: Arc<CustomParameters>,
}

impl JobTemplateOpts {
    pub fn check(
        &self,
        targets: &BTreeMap<ProperDirname, Arc<BenchmarkingTarget>>,
    ) -> Result<JobTemplate> {
        let Self {
            priority,
            initial_boost,
            target_name,
            custom_parameters,
        } = self;

        let target = targets
            .get(target_name)
            .ok_or_else(|| anyhow!("unknown target name {:?}", target_name.as_str()))?;

        let custom_parameters =
            CustomParameters::checked_from(custom_parameters, &target.allowed_custom_parameters)
                .with_context(|| {
                    let context = ron_to_string_pretty(self).expect("no serialisation errors");
                    anyhow!("processing {context}")
                })?;

        Ok(JobTemplate {
            priority: *priority,
            initial_boost: *initial_boost,
            command: target.benchmarking_command.clone_arc(),
            custom_parameters: custom_parameters.into(),
        })
    }
}

/// Settings for calling `evobench-eval`
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalSettings {
    /// Pass the --show-thread-number option to evobench-eval
    /// ("Include the internally-allocated thread number in call
    /// path strings in the output"). Only use if the application
    /// has a limited number of threads (i.e. uses fixed thread
    /// pools); if it allocates new threads all the time then this
    /// will blow up the resulting Excel files.
    pub show_thread_number: bool,
}

/// Direct representation of the evobench-jobs config file
// For why `Arc` is used, see `docs/hacking.md`
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename = "RunConfig")]
pub struct RunConfigOpts {
    pub queues: Arc<QueuesConfig>,

    /// The path to the directory that job runners (`evobench-jobs run
    /// ...`) lock (with error when taken), and for additional files
    /// specific for that instance. By default,
    /// `~/.evobench-jobs/run_jobs_instance`.
    pub run_jobs_instance_basedir: Option<Arc<TildePath<PathBuf>>>,

    /// The path to the directory where `evobench-jobs run daemon` writes logs to. By default,
    /// `~/.evobench-jobs/run_jobs_instance/logs`.
    pub daemon_log_dir: Option<Arc<TildePath<PathBuf>>>,

    pub working_directory_pool: Arc<WorkingDirectoryPoolOpts>,

    /// What command to run on the target project to execute a
    /// benchmarking run; the env variables configured in
    /// CustomParameters are set when running this command.
    pub targets: Vec<Arc<BenchmarkingTarget>>,

    /// A set of named job template lists, referred to by name from
    /// `job_templates_for_insert` and `remote_branch_names_for_poll`.
    /// Each job template in a list generates a separate benchmark run
    /// for each commit that is inserted. The order defines in which
    /// order the jobs are inserted (which means the job generated from
    /// the first template is scheduled first, at least if priorities
    /// are the same). `priority` is added to whatever priority the
    /// inserter asks for, and `initial_boost` is added to the job for
    /// its first run only.
    pub job_template_lists: BTreeMap<KString, Vec<JobTemplateOpts>>,

    /// Job templates for using the "evobench-jobs insert" (or currently
    /// also "insert-local", but this sub-command is planned to be
    /// removed) sub-command. Reference into `job_template_lists` via
    /// `Ref()`, or provide a list of JobTemplate entries directly via
    /// `List()`.
    pub job_templates_for_insert: ValOrRef<JobTemplateListsField, Vec<JobTemplateOpts>>,

    /// Each job receives a copy of these settings after expansion
    pub benchmarking_job_settings: Arc<BenchmarkingJobSettingsOpts>,

    /// Settings for calling `evobench-eval`
    pub eval_settings: Arc<EvalSettings>,

    /// Information on the remote repository of the target project
    pub remote_repository: RemoteRepositoryOpts,

    /// The base of the directory hierarchy where the output files
    /// should be placed. Supports `~/` for specifying the home
    /// directory.
    pub output_base_dir: Arc<TildePath<PathBuf>>,

    /// Optional directory holding directories whose name is taken
    /// from the (optional, depending on the configuration) `DATASET`
    /// custom variable (hacky to mis-use a custom variable for
    /// this?), inside which are directories named after git revision
    /// names (tags or commit ids), the latest which is an ancestor or
    /// the commit itself to be benchmarked,
    /// i.e. `$versioned_datasets_base_dir/$DATASET/$best_rev_name`. The
    /// resolved path (only when both this option and `DATASET` are
    /// provided) is stored in the `DATASET_DIR` env var when calling
    /// the benchmarking entry point of the client app. Supports `~/`
    /// for specifying the home directory.
    pub versioned_datasets_base_dir: Option<Arc<TildePath<PathBuf>>>,

    /// A regular expression matching those Git tags that should be
    /// passed to the target in the `COMMIT_TAGS` env variable (as
    /// comma-separated strings). By default, all tags are passed.
    pub commit_tags_regex: Option<SerializableRegex>,
}

#[derive(Debug)]
pub struct JobTemplateListsField;
impl ValOrRefTarget for JobTemplateListsField {
    fn target_desc() -> Cow<'static, str> {
        "`RunConfig.job_template_lists` field".into()
    }
}

impl DefaultConfigPath for RunConfigOpts {
    fn default_config_file_name_without_suffix() -> Result<Option<ProperFilename>> {
        Ok(Some("evobench-jobs".parse().map_err(|e| anyhow!("{e:#}"))?))
    }
}

/// Checked, produced from `RunConfigOpts`, for docs see there.
pub struct RunConfig {
    pub queues: Arc<QueuesConfig>,
    run_jobs_instance_basedir: Option<PathBuf>,
    daemon_log_dir: Option<PathBuf>,
    pub working_directory_pool: Arc<WorkingDirectoryPoolOpts>,
    // targets: BTreeMap<ProperDirname, Arc<BenchmarkingTarget>>,
    pub job_template_lists: BTreeMap<KString, Arc<[JobTemplate]>>,
    pub job_templates_for_insert: Arc<[JobTemplate]>,
    pub benchmarking_job_settings: Arc<BenchmarkingJobSettingsOpts>,
    pub eval_settings: Arc<EvalSettings>,
    pub remote_repository: RemoteRepository,
    pub output_base_dir: Arc<Path>,
    pub versioned_datasets_base_dir: Option<Arc<Path>>,
    pub targets: BTreeMap<ProperDirname, Arc<BenchmarkingTarget>>,
    pub commit_tags_regex: SerializableRegex,
}

impl RunConfig {
    pub fn run_jobs_instance_basedir(
        &self,
        global_app_state_dir: &GlobalAppStateDir,
    ) -> Result<PathBuf> {
        if let Some(path) = &self.run_jobs_instance_basedir {
            Ok(path.into())
        } else {
            global_app_state_dir.default_run_jobs_instance_basedir()
        }
    }

    pub fn working_directory_change_signals_path(
        &self,
        global_app_state_dir: &GlobalAppStateDir,
    ) -> Result<PathBuf> {
        self.run_jobs_instance_basedir(global_app_state_dir)
            .map(|p| p.append("working_directory_change.signals"))
    }

    pub fn daemon_state_dir(&self, global_app_state_dir: &GlobalAppStateDir) -> Result<PathBuf> {
        self.run_jobs_instance_basedir(global_app_state_dir)
    }

    pub fn daemon_log_dir(&self, global_app_state_dir: &GlobalAppStateDir) -> Result<PathBuf> {
        if let Some(daemon_log_dir) = &self.daemon_log_dir {
            Ok(daemon_log_dir.into())
        } else {
            Ok(self.daemon_state_dir(global_app_state_dir)?.append("logs"))
        }
    }
}

impl RunConfigOpts {
    /// Don't take ownership since RunConfigWithReload can't give it
    pub fn check(&self) -> Result<RunConfig> {
        let RunConfigOpts {
            queues,
            run_jobs_instance_basedir,
            daemon_log_dir,
            working_directory_pool,
            targets,
            job_template_lists,
            job_templates_for_insert,
            benchmarking_job_settings,
            eval_settings,
            remote_repository,
            output_base_dir,
            versioned_datasets_base_dir,
            commit_tags_regex,
        } = self;

        let targets: BTreeMap<ProperDirname, Arc<BenchmarkingTarget>> = {
            let mut seen = BTreeSet::new();
            targets
                .iter()
                .map(|benchmarking_target| {
                    let name = &benchmarking_target.benchmarking_command.target_name;
                    if seen.contains(&name) {
                        bail!("duplicate `target_name` value {:?}", name.as_str())
                    }
                    seen.insert(name);
                    Ok((name.clone(), benchmarking_target.clone_arc()))
                })
                .collect::<Result<_>>()?
        };

        let job_template_lists: BTreeMap<KString, Arc<[JobTemplate]>> = job_template_lists
            .iter()
            .map(
                |(template_list_name, template_list)| -> Result<(KString, Arc<[JobTemplate]>)> {
                    Ok((
                        template_list_name.clone(),
                        template_list
                            .iter()
                            .map(|job_template_opts| job_template_opts.check(&targets))
                            .collect::<Result<_>>()?,
                    ))
                },
            )
            .collect::<Result<_>>()?;

        let job_templates_for_insert = job_templates_for_insert
            // first, make sure owned values are converted, too
            .try_map(|job_template_optss| {
                job_template_optss
                    .iter()
                    .map(|job_template_opts| job_template_opts.check(&targets))
                    .collect::<Result<_>>()
            })?
            // then retrieve the value, either the owned or from the
            // collection
            .value_with_backing(&job_template_lists)?
            // Clone the Arc while it is still alive as a temporary
            .clone_arc();

        let remote_repository = remote_repository.check(&job_template_lists, &targets)?;

        let commit_tags_regex: SerializableRegex =
            if let Some(commit_tags_regex) = commit_tags_regex {
                (*commit_tags_regex).clone()
            } else {
                SerializableRegex::from_str(".*")?
            };

        Ok(RunConfig {
            queues: queues.clone_arc(),
            run_jobs_instance_basedir: run_jobs_instance_basedir
                .as_ref()
                .map(|p| p.resolve())
                .transpose()?
                .map(|r| r.into()),
            daemon_log_dir: daemon_log_dir
                .as_ref()
                .map(|p| p.resolve())
                .transpose()?
                .map(|r| r.into()),
            working_directory_pool: working_directory_pool.clone_arc(),
            job_template_lists,
            job_templates_for_insert,
            benchmarking_job_settings: benchmarking_job_settings.clone_arc(),
            eval_settings: eval_settings.clone_arc(),
            remote_repository,
            output_base_dir: output_base_dir.resolve()?.into(),
            targets,
            versioned_datasets_base_dir: versioned_datasets_base_dir
                .as_ref()
                .map(|d| d.resolve())
                .transpose()?
                .map(Arc::<Path>::from),
            commit_tags_regex,
        })
    }
}

pub struct RunConfigWithReload {
    pub config_file: ConfigFile<RunConfigOpts>,
    pub run_config: RunConfig,
}

impl RunConfigWithReload {
    pub fn load(
        provided_path: Option<Arc<Path>>,
        or_else: impl FnOnce(String) -> Result<RunConfigOpts>,
    ) -> Result<Self> {
        let config_file = ConfigFile::<RunConfigOpts>::load_config(provided_path, or_else)?;
        let run_config = config_file.check()?;
        Ok(Self {
            config_file,
            run_config,
        })
    }

    pub fn perhaps_reload_config(&self) -> Result<Option<Self>> {
        if let Some(config_file) = self.config_file.perhaps_reload_config()? {
            let run_config = config_file.check()?;
            Ok(Some(Self {
                config_file,
                run_config,
            }))
        } else {
            Ok(None)
        }
    }
}
