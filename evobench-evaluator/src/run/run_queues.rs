use std::{
    collections::BTreeSet, convert::Infallible, ops::Neg, path::PathBuf, sync::Arc,
    time::SystemTime,
};

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use itertools::{EitherOrBoth, Itertools};

use crate::{
    ctx,
    date_and_time::time_ranges::{DateTimeRange, LocalNaiveTimeRange},
    info,
    key::RunParameters,
    key_val_fs::{
        key_val::{KeyValConfig, KeyValError, KeyValSync},
        queue::{Queue, QueueGetItemOpts, QueueItem, TimeKey},
    },
    path_util::AppendToPath,
    serde::paths::ProperFilename,
    utillib::logging::{log_level, LogLevel},
};

use super::{
    benchmarking_job::BenchmarkingJob,
    config::{QueuesConfig, ScheduleCondition},
    global_app_state_dir::GlobalAppStateDir,
    run_context::RunContext,
    run_queue::{RunQueue, RunQueueWithNext},
};

// Move, where?
pub fn get_now_chrono() -> DateTime<Local> {
    SystemTime::now().into()
}

pub type Never = Infallible;

#[ouroboros::self_referencing]
pub struct RunQueues {
    pub config: Arc<QueuesConfig>,
    // Checked to be at least 1, at most one is `Immediately`,
    // etc. (private field to prevent by-passing the constructor)
    #[borrows(config)]
    #[covariant]
    pipeline: Vec<RunQueue<'this>>,

    #[borrows(config)]
    #[covariant]
    erroneous_jobs_queue: Option<RunQueue<'this>>,
}

impl RunQueues {
    pub fn pipeline(&self) -> &[RunQueue] {
        self.borrow_pipeline()
    }

    pub fn queue_names(&self) -> Vec<&str> {
        self.pipeline()
            .iter()
            .map(|q| q.file_name.as_str())
            .collect()
    }

    pub fn erroneous_jobs_queue(&self) -> Option<&RunQueue> {
        self.borrow_erroneous_jobs_queue().as_ref()
    }

    pub fn first(&self) -> &RunQueue {
        &self.pipeline()[0]
    }

    /// Also returns the queue following the requested one, if any
    pub fn get_run_queue_with_next_by_name(
        &self,
        file_name: &ProperFilename,
    ) -> Option<RunQueueWithNext> {
        let mut queues = self.pipeline().iter();
        while let Some(current) = queues.next() {
            if current.file_name == *file_name {
                let next = queues.next();
                return Some(RunQueueWithNext { current, next });
            }
        }
        None
    }

    /// The `RunQueue`s paired with their successor (still in the
    /// original, configured, order)
    pub fn run_queue_with_nexts<'s>(&'s self) -> impl Iterator<Item = RunQueueWithNext<'s, 's>> {
        self.pipeline()
            .iter()
            .zip_longest(self.pipeline().iter().skip(1))
            .map(|either_or_both| match either_or_both {
                EitherOrBoth::Both(current, next) => RunQueueWithNext {
                    current,
                    next: Some(next),
                },
                EitherOrBoth::Left(current) => RunQueueWithNext {
                    current,
                    next: None,
                },
                EitherOrBoth::Right(_) => unreachable!("because the left sequence is longer"),
            })
    }

    /// All queues which are runnable at the given time, with their
    /// successor queue, and calculated time window if any
    fn active_queues<'s>(
        &'s self,
        reference_time: DateTime<Local>,
    ) -> impl Iterator<Item = (RunQueueWithNext<'s, 's>, Option<DateTimeRange<Local>>)> {
        self.run_queue_with_nexts()
            .filter_map(move |rq| match rq.schedule_condition {
                ScheduleCondition::Immediately { situation: _ } => Some((rq, None)),
                ScheduleCondition::LocalNaiveTimeWindow {
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
                    let dtr: Option<DateTimeRange<Local>> =
                        ltr.after_datetime(&reference_time, true);
                    if let Some(dtr) = dtr {
                        if dtr.contains(&reference_time) {
                            Some((rq, Some(dtr)))
                        } else {
                            None
                        }
                    } else {
                        info!(
                            "times in {ltr} do not resolve for {reference_time}, \
                             omitting queue {:?}",
                            rq.file_name,
                        );
                        None
                    }
                }
                ScheduleCondition::GraveYard => None,
            })
    }

    /// Run the first or most prioritized job in the queues. Returns
    /// true if a job was found, false if all runnable queues are
    /// empty. This method needs to be run in a loop forever for
    /// daemon style processing. The reason this doesn't do the
    /// looping inside is to allow for a reload of the queue config
    /// and then queues. `current_stop_start`, if given, represents an
    /// active `stop_start` command that was run with `stop` and now
    /// needs a `start` when the next running action does not require
    /// the same command to be `stop`ed. Likewise, this method returns
    /// the active `stop_start` command, if any, by the time it
    /// returns. The caller should pass that back into this method on
    /// the next iteration. Using SliceOrBox to allow carrying it over
    /// a config reload. `now` should be the current time (at least is
    /// understood as such), get it via `get_now_chrono()` right
    /// before calling this method.
    pub fn run_next_job<'s, 'conf, 'r, 'rc>(
        &'s self,
        execute: impl FnMut(RunParameters, &RunQueue) -> Result<()>,
        run_context: &mut RunContext,
        now: DateTime<Local>,
    ) -> Result<bool> {
        let verbose = log_level() >= LogLevel::Info;
        let active_queues: Vec<(RunQueueWithNext<'s, 's>, Option<DateTimeRange<Local>>)> =
            self.active_queues(now).collect();

        let job = {
            // Get the single most prioritized job from each queue (if
            // any). Note: these `QueueItem`s are not locked!
            let mut jobs: Vec<(
                &RunQueueWithNext<'s, 's>,
                Option<DateTimeRange<Local>>,
                QueueItem<BenchmarkingJob>,
                BenchmarkingJob,
            )> = active_queues
                .iter()
                .map(|(rq, dtr)| -> Result<Option<_>> {
                    let mut jobs: Vec<(TimeKey, BenchmarkingJob)> = rq
                        .jobs()
                        .map(|r| {
                            let (item, job) = r?;
                            // Get key and drop item to avoid keeping
                            // open a file handle for every entry in
                            // the queue
                            Ok((item.key()?, job))
                        })
                        .collect::<Result<_, KeyValError>>()
                        .map_err(ctx!("reading entries from queue {:?}", *rq))?;
                    jobs.sort_by_key(|(_, job)| job.priority.neg());

                    if let Some((key, job)) = jobs.into_iter().next() {
                        if let Some(item) = rq.queue.get_item(
                            &key,
                            QueueGetItemOpts {
                                verbose,
                                no_lock: true,
                                error_when_locked: false,
                                delete_first: false,
                            },
                        )? {
                            Ok(Some((rq, (*dtr).clone(), item, job)))
                        } else {
                            info!("entry {key} has disappeared in the mean time, skipping it");
                            Ok(None)
                        }
                    } else {
                        Ok(None)
                    }
                })
                .filter_map(|r| r.transpose())
                .collect::<Result<_>>()?;

            // And then get the most prioritized job of all
            jobs.sort_by_key(|(_, _, _, job)| job.priority.neg());
            jobs.into_iter().next()
        };

        let ran_job = if let Some((rqwn, dtr, item, job)) = job {
            run_context.stop_start_be(rqwn.schedule_condition.stop_start())?;
            if let Some(dtr) = dtr {
                run_context.running_job_in_windowed_queue(&*rqwn, dtr);
            }

            rqwn.run_job(&item, job, self.erroneous_jobs_queue(), execute)?;

            true
        } else {
            run_context.stop_start_be(None)?;

            false
        };

        Ok(ran_job)
    }

    /// Verify that the queue configuration is valid
    fn check_run_queues(&self) -> Result<()> {
        let (pipeline, erroneous_jobs_queue) = (self.pipeline(), self.erroneous_jobs_queue());
        if pipeline.is_empty() {
            bail!(
                "no queues defined -- need at least one, also \
                 suggested is to add a `GraveYard` as the last"
            )
        }
        let mut grave_yard_count = 0;
        let mut seen = BTreeSet::new();
        for run_queue in pipeline {
            let file_name = &run_queue.file_name;
            if seen.contains(file_name) {
                bail!("duplicate queue name {file_name:?}")
            }
            seen.insert(file_name.clone());
            match run_queue.schedule_condition {
                ScheduleCondition::Immediately { situation: _ } => (),
                ScheduleCondition::LocalNaiveTimeWindow {
                    situation: _,
                    stop_start,
                    repeatedly: _,
                    move_when_time_window_ends: _,
                    from: _,
                    to: _,
                } => {
                    if let Some(stop_start) = &stop_start {
                        if stop_start.is_empty() {
                            bail!(
                                "`LocalNaiveTimeWindow.stop_start` was given \
                                 but is the empty list, require at least a program name/path"
                            )
                        }
                    }
                }
                ScheduleCondition::GraveYard => grave_yard_count += 1,
            }
        }
        if let Some(run_queue) = erroneous_jobs_queue.as_ref() {
            let file_name = &run_queue.file_name;
            if seen.contains(file_name) {
                bail!(
                    "duplicate queue name {file_name:?}: `erroneous_jobs_queue` \
                     uses a name also used in the pipeline"
                )
            }
            if !run_queue.schedule_condition.is_grave_yard() {
                bail!("the `erroneous_jobs_queue` must be of kind `GraveYard`")
            }
        }
        if grave_yard_count > 1 {
            bail!("can have at most one `GraveYard` queue");
        }
        if grave_yard_count > 0 {
            if *pipeline
                .last()
                .expect("checked in the if condition")
                .schedule_condition
                != ScheduleCondition::GraveYard
            {
                bail!("`GraveYard` queue must be the last in the pipeline")
            }
        }

        Ok(())
    }

    pub fn open(
        config: Arc<QueuesConfig>,
        create_dirs_if_not_exist: bool,
        global_app_state_dir: &GlobalAppStateDir,
    ) -> Result<Self> {
        let run_queues_basedir =
            config.run_queues_basedir(create_dirs_if_not_exist, global_app_state_dir)?;

        fn make_run_queue<'this>(
            (filename, schedule_condition): &'this (ProperFilename, ScheduleCondition),
            run_queues_basedir: &PathBuf,
            create_dirs_if_not_exist: bool,
        ) -> Result<RunQueue<'this>> {
            let run_queue_path = (&run_queues_basedir).append(filename.as_str());
            Ok(RunQueue {
                file_name: filename.clone(),
                schedule_condition,
                queue: Queue::<BenchmarkingJob>::open(
                    &run_queue_path,
                    KeyValConfig {
                        sync: KeyValSync::All,
                        create_dir_if_not_exists: create_dirs_if_not_exist,
                    },
                )?,
            })
        }

        let slf = Self::try_new(
            config,
            // pipeline:
            |config| -> Result<_> {
                let queues = config
                    .pipeline
                    .iter()
                    .map(|cfg| make_run_queue(cfg, &run_queues_basedir, create_dirs_if_not_exist))
                    .collect::<Result<_>>()?;
                Ok(queues)
            },
            // erroneous_jobs_queue:
            |config| {
                if let Some(cfg) = config.erroneous_jobs_queue.as_ref() {
                    Ok(Some(make_run_queue(
                        cfg,
                        &run_queues_basedir,
                        create_dirs_if_not_exist,
                    )?))
                } else {
                    Ok(None)
                }
            },
        )?;

        slf.check_run_queues()?;

        Ok(slf)
    }
}
