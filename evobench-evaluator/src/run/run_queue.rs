use std::process::Command;

use anyhow::{bail, Result};

use crate::{
    config_file::ron_to_string_pretty,
    ctx, info,
    key_val_fs::{
        key_val::KeyValError,
        queue::{Queue, QueueGetItemOptions, QueueItem, QueueIterationOpts, TimeKey},
    },
    run::{benchmarking_job::BenchmarkingJobState, run_job::JobRunnerWithJob},
    serde::{priority::Priority, proper_filename::ProperFilename},
    utillib::logging::{log_level, LogLevel},
};

use super::{
    benchmarking_job::{BenchmarkingJob, BenchmarkingJobPublic},
    config::ScheduleCondition,
    working_directory_pool::WorkingDirectoryId,
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

/// A loaded copy of the on-disk data, for on-the-fly
/// indexing/multiple traversal
#[derive(Debug, PartialEq)]
pub struct RunQueueData<'conf, 'run_queue> {
    run_queue: &'run_queue RunQueue<'conf>,
    /// The queue items, with total priority from job and queue
    queue_data: Vec<(TimeKey, BenchmarkingJob, Priority)>,
}

impl<'conf, 'run_queue> RunQueueData<'conf, 'run_queue> {
    pub fn run_queue(&self) -> &'run_queue RunQueue<'conf> {
        self.run_queue
    }
    pub fn jobs(&self) -> impl Iterator<Item = &BenchmarkingJob> {
        self.queue_data.iter().map(|(_, job, _)| job)
    }
    /// Priority already includes the queue priority here.
    pub fn entries(&self) -> impl Iterator<Item = &(TimeKey, BenchmarkingJob, Priority)> {
        self.queue_data.iter()
    }
    /// Panics for invalid i
    pub fn entry(&self, i: usize) -> &(TimeKey, BenchmarkingJob, Priority) {
        &self.queue_data[i]
    }
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

    pub fn data<'run_queue>(&'run_queue self) -> Result<RunQueueData<'conf, 'run_queue>> {
        let queue_data = self
            .jobs()
            .map(|r| -> Result<_> {
                let (queue_item, job) = r?;
                let queue_priority = self
                    .schedule_condition
                    .priority()
                    .expect("no graveyard queue in pipeline");
                let priority = (job.priority()? + queue_priority)?;
                Ok((queue_item.key()?, job, priority))
            })
            .collect::<Result<_>>()?;
        Ok(RunQueueData {
            run_queue: self,
            queue_data,
        })
    }

    /// NOTE: this returns unlocked `QueueItem`s! Call
    /// `lock_exclusive()` on them to lock them afterwards.
    // XXX obsolete, kept public and in direct use only for testing, rename and only use in
    pub fn jobs<'s>(
        &'s self,
    ) -> impl Iterator<Item = Result<(QueueItem<'s, BenchmarkingJob>, BenchmarkingJob), KeyValError>>
           + use<'s> {
        let opts = QueueIterationOpts {
            get_item_opts: QueueGetItemOptions {
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

/// A `RunQueueData` paired with its optional successor `RunQueueData` (the
/// queue where jobs go next)
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct RunQueueDataWithNext<'conf, 'run_queue, 'r> {
    pub current: &'r RunQueueData<'conf, 'run_queue>,
    pub next: Option<&'r RunQueueData<'conf, 'run_queue>>,
}

impl<'conf, 'r> RunQueueWithNext<'conf, 'r> {
    /// Run the given job, which must be from this queue. `item`
    /// represents the queue entry of this job, and is used for
    /// locking and deletion--it must not already be locked!
    pub fn run_job(
        &self,
        item: &QueueItem<BenchmarkingJob>,
        job_runner_with_job: &mut JobRunnerWithJob,
        erroneous_jobs_queue: Option<&RunQueue>,
        done_jobs_queue: Option<&RunQueue>,
        working_directory_id: WorkingDirectoryId,
    ) -> Result<()> {
        let _lock = item.lock_exclusive()?;

        let BenchmarkingJobState {
            remaining_count,
            mut remaining_error_budget,
            last_working_directory: _,
        } = job_runner_with_job
            .job_data
            .job
            .benchmarking_job_state
            .clone();

        let job_completed = |remaining_count| -> Result<()> {
            let job = job_runner_with_job
                .job_data
                .job
                .clone_for_queue_reinsertion(BenchmarkingJobState {
                    remaining_count,
                    remaining_error_budget,
                    last_working_directory: Some(working_directory_id),
                });
            info!(
                "job completed: {}",
                ron_to_string_pretty(&job).expect("no err")
            );
            if let Some(done_jobs_queue) = done_jobs_queue {
                done_jobs_queue.push_front(&job)?;
            }
            Ok(())
        };

        let BenchmarkingJobPublic {
            reason,
            // Getting these via job.benchmarking_job_parameters() instead
            run_parameters: _,
            command: _,
        } = job_runner_with_job
            .job_data
            .job
            .benchmarking_job_public
            .clone();

        if remaining_error_budget > 0 {
            if remaining_count > 0 {
                if let Err(error) = job_runner_with_job.run_job(
                    working_directory_id,
                    &reason,
                    &self.current.schedule_condition,
                ) {
                    remaining_error_budget = remaining_error_budget - 1;

                    // XX this should use more important error
                    // logging than info!; (XX also, repetitive
                    // BenchmarkingJob recreation and cloning.)
                    info!(
                        "job gave error: {}: {error:#?}",
                        // XX: give job_runner_ext as the context? And
                        // anyway, todo layered error zones.
                        ron_to_string_pretty(&job_runner_with_job.job_data.job).expect("no err")
                    );
                    if remaining_error_budget > 0 {
                        // Re-schedule
                        let job = job_runner_with_job
                            .job_data
                            .job
                            .clone_for_queue_reinsertion(BenchmarkingJobState {
                                remaining_count,
                                remaining_error_budget,
                                last_working_directory: Some(working_directory_id),
                            });
                        self.current.push_front(&job)?;
                    }
                } else {
                    let remaining_count = remaining_count - 1;
                    if remaining_count > 0 {
                        let maybe_queue;
                        match self.current.schedule_condition {
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
                                    maybe_queue = Some(self.current);
                                } else {
                                    maybe_queue = self.next;
                                }
                            }
                            ScheduleCondition::GraveYard => {
                                unreachable!("already returned at beginning of function")
                            }
                        }

                        let job = job_runner_with_job
                            .job_data
                            .job
                            .clone_for_queue_reinsertion(BenchmarkingJobState {
                                remaining_count,
                                remaining_error_budget,
                                last_working_directory: Some(working_directory_id),
                            });
                        if let Some(queue) = maybe_queue {
                            queue.push_front(&job)?;
                        } else {
                            info!(
                                "job dropping off the pipeline: {}",
                                ron_to_string_pretty(&job).expect("no err")
                            );
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
            let job = job_runner_with_job
                .job_data
                .job
                .clone_for_queue_reinsertion(BenchmarkingJobState {
                    remaining_count,
                    remaining_error_budget,
                    last_working_directory: Some(working_directory_id),
                });

            if let Some(queue) = &erroneous_jobs_queue {
                queue.push_front(&job)?;
            } else {
                info!(
                    "job dropped due to running out of error budget \
                     and no configured erroneous_jobs_queue: {}",
                    ron_to_string_pretty(&job).expect("no err")
                );
            }
        }
        item.delete()?;
        Ok(())
    }

    pub fn handle_timeout(&self) -> Result<()> {
        info!("ran out of time in queue {}", self.current.file_name);
        if self.current.schedule_condition.move_when_time_window_ends() {
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

impl<'conf, 'run_queue, 'r> RunQueueDataWithNext<'conf, 'run_queue, 'r> {
    pub fn run_queue_with_next(&self) -> RunQueueWithNext<'conf, 'run_queue> {
        let RunQueueDataWithNext { current, next } = self;
        let current = current.run_queue();
        let next = next.map(|rq| rq.run_queue());
        RunQueueWithNext { current, next }
    }
}
