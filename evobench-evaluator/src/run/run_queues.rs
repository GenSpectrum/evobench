use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet},
    ops::Neg,
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use itertools::{EitherOrBoth, Itertools};

use crate::{
    ctx,
    date_and_time::time_ranges::{DateTimeRange, LocalNaiveTimeRange},
    git::GitHash,
    info,
    key::RunParameters,
    key_val_fs::{
        key_val::{KeyValConfig, KeyValSync},
        queue::{Queue, QueueGetItemOpts, QueueItem, TimeKey},
    },
    path_util::AppendToPath,
    serde::{priority::Priority, proper_filename::ProperFilename},
    utillib::logging::{log_level, LogLevel},
};

use super::{
    benchmarking_job::BenchmarkingJob,
    config::{BenchmarkingCommand, QueuesConfig, ScheduleCondition},
    global_app_state_dir::GlobalAppStateDir,
    run_context::RunContext,
    run_queue::{RunQueue, RunQueueData, RunQueueDataWithNext, RunQueueWithNext},
};

// Move, where?
pub fn get_now_chrono() -> DateTime<Local> {
    SystemTime::now().into()
}

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

    #[borrows(config)]
    #[covariant]
    done_jobs_queue: Option<RunQueue<'this>>,
}

/// A loaded copy of the on-disk data, for on-the-fly
/// indexing/multiple traversal
pub struct RunQueuesData<'run_queues> {
    run_queues: &'run_queues RunQueues,
    pipeline_data: Vec<RunQueueData<'run_queues, 'run_queues>>,
    /// Value is (index in pipeline_data, index within its queue_data)
    jobs_by_commit_id: BTreeMap<GitHash, Vec<(usize, usize)>>,
}

impl RunQueues {
    pub fn pipeline(&self) -> &[RunQueue] {
        self.borrow_pipeline()
    }

    pub fn erroneous_jobs_queue(&self) -> Option<&RunQueue> {
        self.borrow_erroneous_jobs_queue().as_ref()
    }

    pub fn done_jobs_queue(&self) -> Option<&RunQueue> {
        self.borrow_done_jobs_queue().as_ref()
    }

    pub fn first(&self) -> &RunQueue {
        &self.pipeline()[0]
    }

    pub fn data<'run_queues>(&'run_queues self) -> Result<RunQueuesData<'run_queues>> {
        let pipeline_data: Vec<RunQueueData> = self
            .pipeline()
            .iter()
            .map(|rq| rq.data())
            .collect::<Result<_>>()?;
        let mut jobs_by_commit_id = BTreeMap::default();
        for (i, rqd) in pipeline_data.iter().enumerate() {
            for (j, job) in rqd.jobs().enumerate() {
                let commit_id = &job.benchmarking_job_public.run_parameters.commit_id;
                match jobs_by_commit_id.entry(commit_id.clone()) {
                    Entry::Vacant(vacant_entry) => {
                        vacant_entry.insert(vec![(i, j)]);
                    }
                    Entry::Occupied(mut occupied_entry) => {
                        occupied_entry.get_mut().push((i, j));
                    }
                }
            }
        }
        Ok(RunQueuesData {
            run_queues: self,
            pipeline_data,
            jobs_by_commit_id,
        })
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
    fn run_queue_with_nexts<'s>(&'s self) -> impl Iterator<Item = RunQueueWithNext<'s, 's>> {
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
    fn new_old_active_queues<'s>(
        &'s self,
        reference_time: DateTime<Local>,
    ) -> impl Iterator<Item = (RunQueueWithNext<'s, 's>, Option<DateTimeRange<Local>>)> {
        self.run_queue_with_nexts().filter_map(move |rq| {
            if let Some(range) = rq.current.schedule_condition.is_runnable_at(reference_time) {
                Some((rq, range))
            } else {
                None
            }
        })
    }

    // XXX tmp
    fn active_queues<'s>(
        &'s self,
        reference_time: DateTime<Local>,
    ) -> impl Iterator<Item = (RunQueueWithNext<'s, 's>, Option<DateTimeRange<Local>>)> {
        let new: Vec<_> = self.new_old_active_queues(reference_time).collect();
        let old: Vec<_> = self.old_active_queues(reference_time).collect();
        assert_eq!(new, old);
        new.into_iter()
    }

    /// All queues which are runnable at the given time, with their
    /// successor queue, and calculated time window if any
    fn old_active_queues<'s>(
        &'s self,
        reference_time: DateTime<Local>,
    ) -> impl Iterator<Item = (RunQueueWithNext<'s, 's>, Option<DateTimeRange<Local>>)> {
        self.run_queue_with_nexts()
            .filter_map(move |rq| match rq.current.schedule_condition {
                ScheduleCondition::Immediately { situation: _ } => Some((rq, None)),
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
                            rq.current.file_name,
                        );
                        None
                    }
                }
                ScheduleCondition::GraveYard => None,
            })
    }

    /// The most prioritized job across all runnable queues
    fn old_most_prioritized_job<'s, 'conf, 'r, 'rc>(
        &'s self,
        now: DateTime<Local>,
    ) -> Result<
        Option<(
            RunQueueWithNext<'s, 's>,
            Option<DateTimeRange<Local>>,
            QueueItem<'s, BenchmarkingJob>,
            BenchmarkingJob,
            Priority,
        )>,
    > {
        let verbose = log_level() >= LogLevel::Info;

        // Get the single most prioritized job from each queue (if
        // any). Note: these `QueueItem`s are not locked!
        let mut jobs: Vec<(
            RunQueueWithNext<'s, 's>,
            Option<DateTimeRange<Local>>,
            QueueItem<BenchmarkingJob>,
            BenchmarkingJob,
            Priority,
        )> = self
            .active_queues(now)
            .map(|(rq, dtr)| -> Result<Option<_>> {
                let mut jobs: Vec<(TimeKey, BenchmarkingJob, Priority)> = rq
                    .current
                    .jobs()
                    .map(|r| -> Result<_> {
                        let (item, job) = r?;
                        // Get key and drop item to avoid keeping
                        // open a file handle for every entry in
                        // the queue. Also, pre-calculate
                        // priorities since that can fail.
                        let job_priority = job.priority()?;
                        Ok((item.key()?, job, job_priority))
                    })
                    .collect::<Result<_>>()
                    .map_err(ctx!("reading entries from queue {:?}", rq.current))?;
                jobs.sort_by_key(|(_, _, job_priority)| job_priority.neg());

                if let Some((key, job, job_priority)) = jobs.into_iter().next() {
                    if let Some(item) = rq.current.queue.get_item(
                        &key,
                        QueueGetItemOpts {
                            verbose,
                            no_lock: true,
                            error_when_locked: false,
                            delete_first: false,
                        },
                    )? {
                        let priority = (job_priority
                            + rq.current
                                .schedule_condition
                                .priority()
                                .expect("no graveyards here"))?;
                        Ok(Some((rq, dtr, item, job, priority)))
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

        // And then get the most prioritized job of all, adjusted
        // for the queue it is in.
        jobs.sort_by_key(|(_, _, _, _, priority)| priority.neg());
        Ok(jobs.into_iter().next())
    }

    /// Verify that the queue configuration is valid
    fn check_run_queues(&self) -> Result<()> {
        let pipeline = self.pipeline();
        let erroneous_jobs_queue = self.erroneous_jobs_queue();
        let done_jobs_queue = self.done_jobs_queue();
        if pipeline.is_empty() {
            bail!(
                "no queues defined -- need at least one, also \
                 suggested is to add a `GraveYard` as the last"
            )
        }

        let mut check_seen = {
            let mut seen = BTreeSet::new();
            move |file_name: &ProperFilename| -> Result<()> {
                if seen.contains(file_name) {
                    bail!("duplicate queue name {file_name:?}")
                }
                seen.insert(file_name.clone());
                Ok(())
            }
        };

        let mut grave_yard_count = 0;
        for run_queue in pipeline {
            check_seen(&run_queue.file_name)?;
            match run_queue.schedule_condition {
                ScheduleCondition::Immediately { situation: _ } => (),
                ScheduleCondition::LocalNaiveTimeWindow {
                    priority: _,
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

        let mut check_extra_queue = |name: &str, run_queue: Option<&RunQueue>| -> Result<()> {
            if let Some(run_queue) = run_queue {
                check_seen(&run_queue.file_name)?;
                if !run_queue.schedule_condition.is_grave_yard() {
                    bail!("the `{name}` must be of kind `GraveYard`")
                }
            }
            Ok(())
        };
        check_extra_queue("erroneous_jobs_queue", erroneous_jobs_queue)?;
        check_extra_queue("done_jobs_queue", done_jobs_queue)?;

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
            // done_jobs_queue:
            |config| {
                if let Some(cfg) = config.done_jobs_queue.as_ref() {
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

impl<'run_queues> RunQueuesData<'run_queues> {
    pub fn entries_by_commit_id(
        &self,
        commit_id: &GitHash,
    ) -> impl Iterator<Item = &(TimeKey, BenchmarkingJob, Priority)> {
        let slice = self
            .jobs_by_commit_id
            .get(commit_id)
            .map(|v| v.as_slice())
            .unwrap_or([].as_slice());
        slice
            .iter()
            .copied()
            .map(|(i, j)| self.pipeline_data[i].entry(j))
    }
    pub fn have_entry_with_commit_id(&self, commit_id: &GitHash) -> bool {
        self.entries_by_commit_id(commit_id).next().is_some()
    }

    /// The `RunQueueData`s paired with their successor (still in the
    /// original, configured, order)
    pub fn run_queue_with_nexts<'s>(
        &'s self,
    ) -> impl Iterator<Item = RunQueueDataWithNext<'run_queues, 'run_queues, 's>> {
        self.pipeline_data
            .iter()
            .zip_longest(self.pipeline_data.iter().skip(1))
            .map(|either_or_both| match either_or_both {
                EitherOrBoth::Both(current, next) => RunQueueDataWithNext {
                    current,
                    next: Some(next),
                },
                EitherOrBoth::Left(current) => RunQueueDataWithNext {
                    current,
                    next: None,
                },
                EitherOrBoth::Right(_) => unreachable!("because the left sequence is longer"),
            })
    }

    /// All queues which are runnable at the given time, with their
    /// successor queue, and calculated time window if any
    fn new_active_queues<'s>(
        &'s self,
        reference_time: DateTime<Local>,
    ) -> impl Iterator<
        Item = (
            RunQueueDataWithNext<'run_queues, 'run_queues, 's>,
            Option<DateTimeRange<Local>>,
        ),
    > {
        self.run_queue_with_nexts().filter_map(move |rq| {
            if let Some(range) = rq
                .current
                .run_queue()
                .schedule_condition
                .is_runnable_at(reference_time)
            {
                Some((rq, range))
            } else {
                None
            }
        })
    }

    // XXX tmp
    fn active_queues<'s>(
        &'s self,
        reference_time: DateTime<Local>,
    ) -> impl Iterator<
        Item = (
            RunQueueDataWithNext<'run_queues, 'run_queues, 's>,
            Option<DateTimeRange<Local>>,
        ),
    > {
        let older: Vec<_> = self.run_queues.active_queues(reference_time).collect();
        let new: Vec<_> = self
            .new_active_queues(reference_time)
            .map(|(rq, o)| (rq.run_queue_with_next(), o))
            .collect();
        assert_eq!(older, new);

        self.new_active_queues(reference_time)
    }

    /// The most prioritized job across all runnable queues
    fn most_prioritized_job<'s, 'conf, 'r, 'rc>(
        &'s self,
        now: DateTime<Local>,
    ) -> Result<
        Option<(
            RunQueueDataWithNext<'run_queues, 'run_queues, 's>,
            Option<DateTimeRange<Local>>,
            QueueItem<'run_queues, BenchmarkingJob>,
            &'s BenchmarkingJob,
            Priority,
        )>,
    > {
        let verbose = log_level() >= LogLevel::Info;

        // Get the single most prioritized job from each queue (if
        // any), then of those the most prioritized one. Using
        // min_by_key since this takes the first of the equal jobs,
        // unlike max_by_key.
        if let Some(((key, job, prio), rq, dtr)) = self
            .active_queues(now)
            .filter_map(|(rq, dtr)| -> Option<_> {
                let entry = rq
                    .current
                    .entries()
                    .min_by_key(|(_, _, job_priority)| job_priority.neg())?;

                Some((entry, rq, dtr))
            })
            .min_by_key(|((_, _, job_priority), _, _)| job_priority.neg())
        {
            if let Some(item) = rq.current.run_queue().queue.get_item(
                key,
                QueueGetItemOpts {
                    verbose,
                    no_lock: true,
                    error_when_locked: false,
                    delete_first: false,
                },
            )? {
                Ok(Some((rq, dtr, item, job, *prio)))
            } else {
                info!("entry {key} has disappeared in the mean time, skipping it");
                Ok(None)
            }
        } else {
            Ok(None)
        }
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
        execute: impl FnMut(
            &Option<String>,
            Arc<BenchmarkingCommand>,
            Arc<RunParameters>,
            &RunQueue,
        ) -> Result<()>,
        run_context: &mut RunContext,
        now: DateTime<Local>,
    ) -> Result<bool> {
        // XXX remove old_* stuff check once verified.
        let old_job = self.run_queues.old_most_prioritized_job(now)?;
        let old_job_1 = old_job
            .as_ref()
            .map(|(rqwn, dtr, item, job, prio)| (rqwn.clone(), dtr, item, job, prio));
        let job = self.most_prioritized_job(now)?;
        let job_1 = job.as_ref().map(|(rqwn, dtr, item, job, prio)| {
            (rqwn.run_queue_with_next(), dtr, item, *job, prio)
        });
        assert_eq!(old_job_1, job_1);

        let ran_job = if let Some((rqdwn, dtr, item, job, _)) = job {
            let rq = rqdwn.current.run_queue();
            run_context.stop_start_be(rq.schedule_condition.stop_start())?;
            if let Some(dtr) = dtr {
                run_context.running_job_in_windowed_queue(rq, dtr);
            }

            rqdwn.run_queue_with_next().run_job(
                &item,
                job.clone(),
                self.run_queues.erroneous_jobs_queue(),
                self.run_queues.done_jobs_queue(),
                execute,
            )?;

            true
        } else {
            run_context.stop_start_be(None)?;

            false
        };

        Ok(ran_job)
    }
}
