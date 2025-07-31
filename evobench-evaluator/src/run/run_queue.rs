use std::{ops::Deref, process::Command, sync::Arc};

use anyhow::{bail, Result};

use crate::{
    ctx, info,
    key::RunParameters,
    key_val_fs::{
        key_val::KeyValError,
        queue::{Queue, QueueGetItemOpts, QueueItem, QueueIterationOpts},
    },
    serde::proper_filename::ProperFilename,
    utillib::{
        arc::CloneArc,
        logging::{log_level, LogLevel},
    },
};

use super::{
    benchmarking_job::{BenchmarkingJob, BenchmarkingJobPublic},
    config::{BenchmarkingCommand, ScheduleCondition},
};

// Move, where?
pub fn run_command(cmd: &[String], start_stop: &str) -> Result<()> {
    assert!(
        !cmd.is_empty(),
        "start_stop should have been checked in `check_run_queues` already"
    );
    let mut cmd: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    cmd.push(start_stop);
    info!("running command {cmd:?}");
    let mut command = Command::new(cmd[0]);
    command.args(&cmd[1..]);
    // XX consistent capture?
    let status = command.status().map_err(ctx!("running {cmd:?}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("command {cmd:?} gave status {status}")
    }
}

#[derive(Debug, PartialEq)]
pub struct RunQueue<'conf> {
    pub file_name: ProperFilename,
    pub schedule_condition: &'conf ScheduleCondition,
    pub queue: Queue<BenchmarkingJob>,
}

pub enum TerminationReason {
    Timeout,
    QueueEmpty,
    GraveYard,
}

impl<'conf> RunQueue<'conf> {
    pub fn push_front(&self, job: &BenchmarkingJob) -> Result<(), KeyValError> {
        self.queue.push_front(job)
    }

    /// NOTE: this returns unlocked `QueueItem`s! Call
    /// `lock_exclusive()` on them to lock them afterwards.
    pub fn jobs<'s>(
        &'s self,
    ) -> impl Iterator<Item = Result<(QueueItem<'s, BenchmarkingJob>, BenchmarkingJob), KeyValError>>
           + use<'s> {
        let opts = QueueIterationOpts {
            get_item_opts: QueueGetItemOpts {
                no_lock: true,
                error_when_locked: false,
                verbose: log_level() >= LogLevel::Info,
                delete_first: false,
            },
            wait: false,
            stop_at: None,
            reverse: false,
        };
        self.queue.items(opts)
    }
}

/// A `RunQueue` paired with its optional successor `RunQueue` (the
/// queue where jobs go next)
#[derive(Debug, PartialEq, Clone, Copy)]
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

impl<'conf, 'r> RunQueueWithNext<'conf, 'r> {
    /// Run the given job, which must be from this queue. `item`
    /// represents the queue entry of this job, and is used for
    /// locking and deletion--it must not already be locked!
    pub fn run_job(
        &self,
        item: &QueueItem<BenchmarkingJob>,
        job: BenchmarkingJob,
        erroneous_jobs_queue: Option<&RunQueue>,
        done_jobs_queue: Option<&RunQueue>,
        mut execute: impl FnMut(
            &Option<String>,
            Arc<BenchmarkingCommand>,
            Arc<RunParameters>,
            &RunQueue,
        ) -> Result<()>,
    ) -> Result<()> {
        let _lock = item.lock_exclusive()?;

        let BenchmarkingJobPublic {
            remaining_count,
            mut remaining_error_budget,
            reason,
            run_parameters,
            command,
        } = job.benchmarking_job_public.clone();

        let job_completed = |remaining_count| -> Result<()> {
            let mut job = job.clone_for_queue_reinsertion();
            job.benchmarking_job_public.remaining_count = remaining_count;
            job.benchmarking_job_public.remaining_error_budget = remaining_error_budget;

            info!("job completed: {job:?}");
            if let Some(done_jobs_queue) = done_jobs_queue {
                done_jobs_queue.push_front(&job)?;
            }
            Ok(())
        };

        if remaining_error_budget > 0 {
            if remaining_count > 0 {
                if let Err(error) = execute(
                    &reason,
                    command.clone_arc(),
                    run_parameters.clone_arc(),
                    self.current,
                ) {
                    remaining_error_budget = remaining_error_budget - 1;

                    let mut job = job.clone_for_queue_reinsertion();
                    job.benchmarking_job_public.remaining_count = remaining_count;
                    job.benchmarking_job_public.remaining_error_budget = remaining_error_budget;

                    // XX this should use more important error
                    // logging than info!; (XX also, repetitive
                    // BenchmarkingJob recreation and cloning.)
                    info!("job gave error: {job:?}: {error:#?}");
                    if remaining_error_budget > 0 {
                        // Re-schedule
                        self.push_front(&job)?;
                    }
                } else {
                    let remaining_count = remaining_count - 1;
                    if remaining_count > 0 {
                        let maybe_queue;
                        match self.schedule_condition {
                            ScheduleCondition::Immediately { situation: _ } => {
                                // Job is always going to the next queue
                                maybe_queue = self.next;
                            }
                            ScheduleCondition::LocalNaiveTimeWindow {
                                priority: _,
                                situation: _,
                                stop_start: _,
                                repeatedly,
                                move_when_time_window_ends: _,
                                from: _,
                                to: _,
                            } => {
                                if *repeatedly {
                                    // Job is going to the current queue (as
                                    // long as `to` has not been reached,
                                    // otherwise the next queue, but then will
                                    // move them all anyway once running out,
                                    // so doesn't matter, and won't parse `to`
                                    // time here, because need to do that
                                    // before we start, hence using `stop_at`
                                    // for that. Thus, simply:)
                                    maybe_queue = Some(self);
                                } else {
                                    maybe_queue = self.next;
                                }
                            }
                            ScheduleCondition::GraveYard => {
                                unreachable!("already returned at beginning of function")
                            }
                        }

                        let mut job = job.clone_for_queue_reinsertion();
                        job.benchmarking_job_public.remaining_count = remaining_count;
                        job.benchmarking_job_public.remaining_error_budget = remaining_error_budget;

                        if let Some(queue) = maybe_queue {
                            queue.push_front(&job)?;
                        } else {
                            info!("job dropping off the pipeline: {job:?}");
                        }
                    } else {
                        job_completed(remaining_count)?;
                    }
                }
            } else {
                info!(
                    "should never get here normally: job stored in normal queue \
                     with remaining_count 0"
                );
                job_completed(remaining_count)?;
            }
        }
        if remaining_error_budget == 0 {
            let mut job = job.clone_for_queue_reinsertion();
            job.benchmarking_job_public.remaining_count = remaining_count;
            job.benchmarking_job_public.remaining_error_budget = remaining_error_budget;

            if let Some(queue) = &erroneous_jobs_queue {
                queue.push_front(&job)?;
            } else {
                info!(
                    "job dropped due to running out of error budget \
                     and no configured erroneous_jobs_queue: {job:?}"
                );
            }
        }
        item.delete()?;
        Ok(())
    }

    pub fn handle_timeout(&self) -> Result<()> {
        info!("ran out of time in queue {}", self.file_name);
        if self.schedule_condition.move_when_time_window_ends() {
            let mut count = 0;
            for entry in self.current.queue.sorted_entries(false, None, false)? {
                // XX continue in the face of
                // errors? Just globally in
                // the queue?
                let mut entry = entry?;
                let val = entry.get()?;
                if let Some(next) = self.next {
                    next.push_front(&val)?;
                }
                entry.delete()?;
                count += 1;
            }
            info!(
                "moved {count} entries to queue {:?}",
                self.next.map(|q| &q.file_name)
            );
        }
        Ok(())
    }
}
