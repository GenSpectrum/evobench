use std::time::SystemTime;

use anyhow::Result;

use crate::{
    info_if,
    key::RunParameters,
    key_val_fs::{
        key_val::KeyValError,
        queue::{Queue, QueueIterationOpts},
    },
    serde::paths::ProperFilename,
};

use super::{benchmarking_job::BenchmarkingJob, config::ScheduleCondition};

#[derive(Debug)]
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

    pub fn run<'s>(
        &'s self,
        wait: bool,
        verbose: bool,
        stop_at: Option<SystemTime>,
        // Have to give ownership to CheckedRunParameters, don't
        // understand why.
        mut execute: impl FnMut(RunParameters) -> Result<()>,
        next_queue: Option<&Self>,
        // Where jobs go when they run out of error budget
        erroneous_jobs_queue: Option<&Self>,
    ) -> Result<TerminationReason>
    where
        'conf: 's,
    {
        if *self.schedule_condition == ScheduleCondition::GraveYard {
            info_if!(verbose, "skip running jobs in the GraveYard");
            return Ok(TerminationReason::GraveYard);
        }

        let opts = QueueIterationOpts {
            no_lock: false,
            error_when_locked: false,
            wait,
            stop_at,
            verbose,
            delete_first: false,
        };
        let items = self.queue.items(opts);
        for item_and_value in items {
            if let Some(stop_at) = stop_at {
                let now = SystemTime::now();
                if now >= stop_at {
                    info_if!(verbose, "reached timeout time {stop_at:?}");
                    return Ok(TerminationReason::Timeout);
                }
            }

            let (mut item, queue_arguments) = item_and_value?;
            let BenchmarkingJob {
                run_parameters,
                remaining_count,
                mut remaining_error_budget,
            } = queue_arguments;
            if remaining_error_budget > 0 {
                if remaining_count > 0 {
                    if let Err(error) = execute(run_parameters.clone()) {
                        remaining_error_budget = remaining_error_budget - 1;
                        // XX this should use more important error
                        // logging than info!; (XX also, repetitive
                        // BenchmarkingJob recreation and cloning.)
                        let job = BenchmarkingJob {
                            run_parameters: run_parameters.clone(),
                            remaining_count,
                            remaining_error_budget,
                        };
                        info_if!(verbose, "job gave error: {job:?}: {error:#?}");
                        if remaining_error_budget > 0 {
                            // Re-schedule
                            self.push_front(&job)?;
                        }
                    } else {
                        let remaining_count = remaining_count - 1;
                        if remaining_count > 0 {
                            let maybe_queue;
                            match self.schedule_condition {
                                ScheduleCondition::Immediately => {
                                    // Job is always going to the next queue
                                    maybe_queue = next_queue;
                                }
                                ScheduleCondition::LocalNaiveTimeRange {
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
                                        maybe_queue = next_queue;
                                    }
                                }
                                ScheduleCondition::GraveYard => {
                                    unreachable!("already returned at beginning of function")
                                }
                            }

                            let job = BenchmarkingJob {
                                run_parameters: run_parameters.clone(),
                                remaining_count,
                                remaining_error_budget,
                            };

                            if let Some(queue) = maybe_queue {
                                queue.push_front(&job)?;
                            } else {
                                info_if!(verbose, "job dropping off the pipeline: {job:?}");
                            }
                        }
                    }
                }
            }
            if remaining_error_budget == 0 {
                let job = BenchmarkingJob {
                    run_parameters,
                    remaining_count,
                    remaining_error_budget,
                };

                if let Some(queue) = &erroneous_jobs_queue {
                    queue.push_front(&job)?;
                } else {
                    info_if!(
                        verbose,
                        "job dropped due to running out of error budget \
                         and no configured erroneous_jobs_queue: {job:?}"
                    );
                }
            }
            item.delete()?;
        }
        Ok(TerminationReason::QueueEmpty)
    }
}
