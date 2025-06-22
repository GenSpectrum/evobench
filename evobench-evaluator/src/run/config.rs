use std::{collections::BTreeMap, fmt::Debug, path::PathBuf};

use anyhow::Result;

use crate::{
    config_file::LoadConfigFile,
    io_util::create_dir_if_not_exists,
    path_util::AppendToPath,
    serde::{date_and_time::LocalNaiveTime, paths::ProperFilename},
    utillib::home::home_dir,
};

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScheduleCondition {
    /// Run jobs in this queue once right away
    Immediately,

    /// Run jobs in this queue between the given times on every
    /// day. After the time window runs out, remaining jobs in the
    /// queue are moved to the next queue (or are dropped if there is
    /// none).
    LocalNaiveTimeRange {
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
        move_on_timeout: bool,
        /// Times in the local time zone, scheduled to run every
        /// day--except if one of the two times is not unambiguous on
        /// a given day (e.g. due to DST changes), the whole queue is
        /// not scheduled on that day.
        from: LocalNaiveTime,
        to: LocalNaiveTime,
    },

    /// A queue that is never run and never emptied, to add to the end
    /// of the queue pipeline to take up jobs that have been expelled
    /// from the second last queue, for informational purposes.
    GraveYard,
}

impl ScheduleCondition {
    pub fn time_range(&self) -> Option<(LocalNaiveTime, LocalNaiveTime)> {
        match self {
            ScheduleCondition::Immediately => None,
            ScheduleCondition::LocalNaiveTimeRange {
                stop_start: _,
                repeatedly: _,
                move_on_timeout: _,
                from,
                to,
            } => Some((from.clone(), to.clone())),
            ScheduleCondition::GraveYard => None,
        }
    }

    /// Returns true if the condition offers that flag *and* it is true
    pub fn move_on_timeout(&self) -> bool {
        match self {
            ScheduleCondition::Immediately => false,
            ScheduleCondition::LocalNaiveTimeRange {
                stop_start: _,
                repeatedly: _,
                move_on_timeout,
                from: _,
                to: _,
            } => *move_on_timeout,
            ScheduleCondition::GraveYard => false,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuesConfig {
    /// If not given, `~/.evobench-run-queues/` is used. Also used for
    /// locking the `run` action of evobench-run, to ensure only one
    /// benchmarking job is executed at the same time--if you
    /// configure multiple such directories then you don't have this
    /// guarantee any more.
    pub run_queues_basedir: Option<PathBuf>,

    /// The queues to use (file names, without '/'), and their
    /// scheduled execution condition
    pub queues: Vec<(ProperFilename, ScheduleCondition)>,
}

/// Direct representation of the evobench-run config file
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    #[serde(flatten)]
    pub queues_config: QueuesConfig,

    /// The key names (environment variable names) that are allowed
    /// (value `false`) or required (value `true`) for benchmarking
    /// the given project
    pub custom_parameters_required: BTreeMap<String, bool>,
}

impl QueuesConfig {
    pub fn _run_queues_basedir(&self) -> Result<PathBuf> {
        if let Some(path) = &self.run_queues_basedir {
            Ok(path.into())
        } else {
            let home = home_dir()?;
            Ok(home.append(".evobench-run-queues"))
        }
    }
    pub fn run_queues_basedir(&self, create_if_not_exists: bool) -> Result<PathBuf> {
        let base_dir = self._run_queues_basedir()?;
        if create_if_not_exists {
            create_dir_if_not_exists(&base_dir, "queues base directory")?;
        }
        Ok(base_dir)
    }
}

impl LoadConfigFile for RunConfig {
    fn default_config_path_without_suffix() -> Result<Option<PathBuf>> {
        let home = home_dir()?;
        Ok(Some(home.append(".evobench-run")))
    }
}
