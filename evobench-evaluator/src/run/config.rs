use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{anyhow, Result};

use crate::{
    config_file::DefaultConfigPath,
    io_utils::{bash::cmd_as_bash_string, div::create_dir_if_not_exists},
    key::CustomParametersSetOpts,
    serde::{
        date_and_time::LocalNaiveTime, git_branch_name::GitBranchName, git_url::GitUrl,
        priority::Priority, proper_filename::ProperFilename,
    },
};

use super::{
    benchmarking_job::BenchmarkingJobSettingsOpts, global_app_state_dir::GlobalAppStateDir,
    run_queues::RunQueues, working_directory_pool::WorkingDirectoryPoolOpts,
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
    GraveYard,
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
                    cmd_as_bash_string(st)
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
            ScheduleCondition::GraveYard => f.write_str("GraveYard"),
        }
    }
}

impl ScheduleCondition {
    /// Whether this queue will never run its jobs
    pub fn is_grave_yard(&self) -> bool {
        match self {
            ScheduleCondition::GraveYard => true,
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
            ScheduleCondition::GraveYard => None,
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
            ScheduleCondition::GraveYard => None,
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
            ScheduleCondition::GraveYard => false,
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
            ScheduleCondition::GraveYard => None,
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
            } => Some(priority.unwrap_or(RunQueues::TIMED_QUEUE_DEFAULT_PRIORITY)),
            ScheduleCondition::GraveYard => None,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuesConfig {
    /// If not given, `~/.evobench-run/queues/` is used. Also used for
    /// locking the `run` action of evobench-run, to ensure only one
    /// benchmarking job is executed at the same time--if you
    /// configure multiple such directories then you don't have this
    /// guarantee any more.
    pub run_queues_basedir: Option<PathBuf>,

    /// The queues to use (file names, without '/'), and their
    /// scheduled execution condition
    pub pipeline: Vec<(ProperFilename, ScheduleCondition)>,

    /// The queue where to put jobs when they run out of
    /// `error_budget` (if `None` is given, the jobs will be dropped--
    /// silently unless verbose flag is given). Should be of
    /// scheduling type GraveYard (or perhaps a future messaging
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
            if create_if_not_exists {
                create_dir_if_not_exists(base_dir, "queues base directory")?;
            }
            Ok(base_dir.into())
        } else {
            global_app_state_dir.run_queues_basedir()
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RemoteRepository {
    /// The Git repository to clone the target project from
    pub url: GitUrl,

    /// The remote branches to track
    pub remote_branch_names: Vec<GitBranchName>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
/// What command to run on the target project to execute a
/// benchmarking run; the env variables configured in CustomParameters
/// are set when running this command.
pub struct BenchmarkingCommand {
    /// Relative path to the subdirectory (provide "." for the top
    /// level of the working directory) where to run the command
    pub subdir: PathBuf,

    /// Name or path to the command to run, e.g. "make"
    pub command: PathBuf,

    /// Arguments to the command, e.g. "bench"
    pub arguments: Vec<String>,
}

/// Direct representation of the evobench-run config file
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    // Usage for reloads dictate the Arc (with the current approaches,
    // which needs to leave the config intact while taking a shared
    // reference to the QueuesConfig because both parts are moved, in
    // evobench-run).
    pub queues: Arc<QueuesConfig>,

    // same as above re Arc use
    pub working_directory_pool: Arc<WorkingDirectoryPoolOpts>,

    /// Information on the remote repository of the target project
    pub remote_repository: RemoteRepository,

    /// The key names (environment variable names) that are allowed
    /// (value `false`) or required (value `true`) for benchmarking
    /// the given project
    pub custom_parameters_required: BTreeMap<String, bool>,

    /// The set of key/value pairs (which must conform to
    /// `custom_parameters_required`) that should be tested.
    pub custom_parameters_set: CustomParametersSetOpts,

    pub benchmarking_job_settings: BenchmarkingJobSettingsOpts,

    pub benchmarking_command: BenchmarkingCommand,

    /// The base of the directory hierarchy where the output files
    /// should be placed
    pub output_base_dir: PathBuf,
}

impl DefaultConfigPath for RunConfig {
    fn default_config_file_name_without_suffix() -> Result<Option<ProperFilename>> {
        Ok(Some("evobench-run".parse().map_err(|e| anyhow!("{e}"))?))
    }
}
