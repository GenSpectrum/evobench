use std::{
    convert::Infallible,
    ops::Deref,
    path::PathBuf,
    sync::Arc,
    thread::sleep,
    time::{Duration, SystemTime},
};

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use itertools::{EitherOrBoth, Itertools};

use crate::{
    info_if,
    key::RunParameters,
    key_val_fs::{
        key_val::{KeyValConfig, KeyValSync},
        queue::Queue,
    },
    path_util::AppendToPath,
    run::run_queue::TerminationReason,
    serde::{date_and_time::LocalNaiveTime, paths::ProperFilename},
};

use super::{
    benchmarking_job::BenchmarkingJob,
    config::{QueuesConfig, ScheduleCondition},
    run_queue::RunQueue,
};

/// A `RunQueue` paired with its optional successor `RunQueue` (the
/// queue where jobs go next)
#[derive(Debug, Clone, Copy)]
pub struct RunQueueWithNext<'conf, 'r> {
    pub current: &'r RunQueue<'conf>,
    pub next: Option<&'r RunQueue<'conf>>,
}

impl<'conf, 'r> Deref for RunQueueWithNext<'conf, 'r> {
    type Target = RunQueue<'conf>;

    fn deref(&self) -> &Self::Target {
        &self.current
    }
}

pub type Never = Infallible;

#[ouroboros::self_referencing]
pub struct RunQueues {
    pub config: Arc<QueuesConfig>,
    // Checked to be at least 1, at most one is `Immediately`,
    // etc. (private field to prevent by-passing the constructor)
    #[borrows(config)]
    #[covariant]
    run_queues: Vec<RunQueue<'this>>,

    #[borrows(config)]
    #[covariant]
    erroneous_jobs_queue: Option<RunQueue<'this>>,
}

impl RunQueues {
    pub fn run_queues(&self) -> &[RunQueue] {
        self.borrow_run_queues()
    }

    pub fn queue_names(&self) -> Vec<&str> {
        self.run_queues()
            .iter()
            .map(|q| q.file_name.as_str())
            .collect()
    }

    pub fn erroneous_jobs_queue(&self) -> Option<&RunQueue> {
        self.borrow_erroneous_jobs_queue().as_ref()
    }

    pub fn first(&self) -> &RunQueue {
        &self.run_queues()[0]
    }

    /// Also returns the queue following the requested one, if any
    pub fn get_run_queue_by_name(
        &self,
        file_name: &ProperFilename,
    ) -> Option<(&RunQueue, Option<&RunQueue>)> {
        let mut queues = self.run_queues().iter();
        while let Some(run_queue) = queues.next() {
            if &run_queue.file_name == file_name {
                let next_queue = queues.next();
                return Some((run_queue, next_queue));
            }
        }
        None
    }

    /// The `RunQueue`s paired with their successor (still in the
    /// original, configured, order)
    pub fn run_queue_with_nexts<'s>(&'s self) -> Vec<RunQueueWithNext<'s, 's>> {
        self.run_queues()
            .iter()
            .zip_longest(self.run_queues().iter().skip(1))
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
            .collect()
    }

    /// The queues with `ScheduleCondition::Immediately`, and those
    /// with time ranges, as separate vectors (no other queues with
    /// runnable jobs than those two groups currently exist). The
    /// immediate group is in the original sort order, the group with
    /// time ranges is sorted by start time, ready to process
    /// sequentially.
    fn immediate_and_ranged_queues(
        &self,
    ) -> (
        Vec<RunQueueWithNext>,
        Vec<((LocalNaiveTime, LocalNaiveTime), RunQueueWithNext)>,
    ) {
        let mut immediate_queues: Vec<RunQueueWithNext> = Vec::new();
        let mut ranged_queues: Vec<((LocalNaiveTime, LocalNaiveTime), RunQueueWithNext)> =
            Vec::new();
        let mut other: Vec<RunQueueWithNext> = Vec::new();

        for q in self.run_queue_with_nexts() {
            if *q.schedule_condition == ScheduleCondition::Immediately {
                immediate_queues.push(q);
            } else if let Some(range) = q.schedule_condition.time_range() {
                ranged_queues.push((range, q));
            } else if *q.schedule_condition == ScheduleCondition::GraveYard {
                // Not scheduling jobs from this queue
                ()
            } else {
                other.push(q);
            }
        }

        if !other.is_empty() {
            // XXX should instead add a method to do the above ^, checked statically!
            unreachable!("queues I don't know how to schedule: {other:?}")
        }

        ranged_queues.sort_by_key(|((start, _), _)| start.clone());

        (immediate_queues, ranged_queues)
    }

    /// Run jobs in the queues, for one cycle; this needs to be run in
    /// a loop forever for daemon style processing. The reason this
    /// doesn't do the looping inside is to allow for a reload of the
    /// queue config and then queues.
    pub fn run<'conf>(
        &self,
        verbose: bool,
        mut execute: impl FnMut(RunParameters) -> Result<()>,
    ) -> Result<()> {
        let (immediate_queues, ranged_queues_by_time) = self.immediate_and_ranged_queues();

        // Immediate queries always have priority, regardless of
        // concurrent ranged queues. Do not pass a timeout.
        for q in &immediate_queues {
            q.run(
                false,
                verbose,
                None,
                &mut execute,
                q.next,
                self.erroneous_jobs_queue(),
            )?;
        }

        for ((from, to), q) in &ranged_queues_by_time {
            let now_system = SystemTime::now();
            let now_chrono = DateTime::<Local>::from(now_system);
            let now = now_chrono.naive_local();
            info_if!(
                verbose,
                "it is now {now_chrono:?}, {now} -- \
                     checking queue {} with time range {from}..{to}",
                q.file_name
            );
            if let Some((from, to)) = (|| -> Option<_> {
                let from = from.with_date_as_unambiguous_local(now.date())?;
                let to = to.with_date_as_unambiguous_local(now.date())?;
                Some((from, to))
            })() {
                if from <= now_chrono && now_chrono < to {
                    info_if!(
                        verbose,
                        "it is now {now_chrono:?}, {now} -> processing queue {}",
                        q.file_name
                    );
                    match q.run(
                        false,
                        verbose,
                        Some(to.into()),
                        &mut execute,
                        q.next,
                        self.erroneous_jobs_queue(),
                    )? {
                        TerminationReason::Timeout => {
                            info_if!(verbose, "ran out of time in queue {}", q.file_name);
                            if q.schedule_condition.move_on_timeout() {
                                let mut count = 0;
                                for entry in q.current.queue.sorted_entries(false, None) {
                                    // XX continue in the face of
                                    // errors? Just globally in
                                    // the queue?
                                    let mut entry = entry?;
                                    let val = entry.get()?;
                                    if let Some(next) = q.next {
                                        next.push_front(&val)?;
                                    }
                                    entry.delete()?;
                                    count += 1;
                                }
                                info_if!(
                                    verbose,
                                    "moved {count} entries to queue {:?}",
                                    q.next.map(|q| &q.file_name)
                                );
                            }
                        }
                        TerminationReason::QueueEmpty => (),
                        TerminationReason::GraveYard => unreachable!("not a ranged queue"),
                    }
                }
            }
        }
        sleep(Duration::from_secs(5));
        Ok(())
    }

    pub fn check_run_queues(run_queues: &Vec<RunQueue>) -> Result<()> {
        if run_queues.is_empty() {
            bail!(
                "no queues defined -- need at least one, also \
                 suggested is to add a `GraveYard` as the last"
            )
        }
        let mut grave_yard_count = 0;
        for run_queue in run_queues {
            match run_queue.schedule_condition {
                ScheduleCondition::Immediately => (),
                ScheduleCondition::LocalNaiveTimeRange {
                    stop_start: _,
                    repeatedly: _,
                    move_on_timeout: _,
                    from: _,
                    to: _,
                } => (),
                ScheduleCondition::GraveYard => grave_yard_count += 1,
            }
        }
        if grave_yard_count > 1 {
            bail!("can have at most one `GraveYard` queue");
        }
        if grave_yard_count > 0 {
            if *run_queues
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

    pub fn open(config: Arc<QueuesConfig>, create_dirs_if_not_exist: bool) -> Result<Self> {
        let run_queues_basedir = config.run_queues_basedir(create_dirs_if_not_exist)?;

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

        Self::try_new(
            config,
            // run_queues:
            |config| {
                let queues = config
                    .pipeline
                    .iter()
                    .map(|cfg| make_run_queue(cfg, &run_queues_basedir, create_dirs_if_not_exist))
                    .collect::<Result<_>>()?;
                Self::check_run_queues(&queues)?;
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
        )
    }
}
