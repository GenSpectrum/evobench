use std::{process::Command, time::SystemTime};

use anyhow::{bail, Result};

use crate::{
    ctx,
    date_and_time::system_time_with_display::SystemTimeWithDisplay,
    info,
    key::RunParameters,
    key_val_fs::{
        key_val::KeyValError,
        queue::{Queue, QueueIterationOpts},
    },
    serde::paths::ProperFilename,
    utillib::{
        logging::{log_level, LogLevel},
        slice_or_box::SliceOrBox,
    },
};

use super::{benchmarking_job::BenchmarkingJob, config::ScheduleCondition};

fn run_command(cmd: &[String], start_stop: &str) -> Result<()> {
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

pub fn perhaps_run_current_stop_start(perhaps_cmd: Option<SliceOrBox<String>>) -> Result<()> {
    if let Some(cmd) = perhaps_cmd {
        run_command(&cmd, "start")?;
    }
    Ok(())
}

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

    pub fn stop_start(&self) -> Option<&'conf [String]> {
        self.schedule_condition.stop_start()
    }

    /// For `current_stop_start` see `RunQueues.run()`. Also returns
    /// how many jobs were *handled* (not necessarily run, could have
    /// just been moved to other queues / rescheduled), and the reason
    /// for returning.
    pub fn run<'s>(
        &'s self,
        wait: bool,
        stop_at: Option<SystemTime>,
        // Have to give ownership to CheckedRunParameters, don't
        // understand why.
        mut execute: impl FnMut(RunParameters) -> Result<()>,
        next_queue: Option<&Self>,
        // Where jobs go when they run out of error budget
        erroneous_jobs_queue: Option<&Self>,
        mut current_stop_start: Option<SliceOrBox<'conf, String>>,
    ) -> Result<(Option<SliceOrBox<'conf, String>>, usize, TerminationReason)>
    where
        'conf: 's,
    {
        if *self.schedule_condition == ScheduleCondition::GraveYard {
            info!("skip running jobs in the GraveYard");
            return Ok((current_stop_start, 0, TerminationReason::GraveYard));
        }

        let opts = QueueIterationOpts {
            no_lock: false,
            error_when_locked: false,
            wait,
            stop_at,
            verbose: log_level() >= LogLevel::Info,
            delete_first: false,
        };
        let items = self.queue.items(opts);
        let mut handled_command = false;
        let mut num_jobs_handled = 0;
        for item_and_value in items {
            if !handled_command {
                // Do the stopping or starting as appropriate for the
                // new queue context
                if let Some(new_css) = self.stop_start() {
                    if let Some(old_css) = &current_stop_start {
                        if new_css == &**old_css {
                            info!("no change in stop_start command, leave it as is");
                        } else {
                            info!("change in stop_start command: end the previous period");
                            run_command(&old_css, "start")?;
                            info!("change in stop_start command: begin the new period");
                            run_command(new_css, "stop")?;
                            current_stop_start = Some(new_css.into());
                        }
                    } else {
                        info!("change in stop_start command: begin new period");
                        run_command(new_css, "stop")?;
                        current_stop_start = Some(new_css.into());
                    }
                } else {
                    if let Some(old_css) = current_stop_start {
                        info!("change in stop_start command: end previous period");
                        run_command(&old_css, "start")?;
                        current_stop_start = None;
                    }
                }

                handled_command = true;
            }

            if let Some(stop_at) = stop_at {
                let stop_at = SystemTimeWithDisplay(stop_at);
                let now = SystemTimeWithDisplay(SystemTime::now());
                if now >= stop_at {
                    info!("reached timeout time {stop_at}");
                    return Ok((
                        current_stop_start,
                        num_jobs_handled,
                        TerminationReason::Timeout,
                    ));
                }
            }

            let (mut item, queue_arguments) = item_and_value?;
            num_jobs_handled += 1;
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
                                ScheduleCondition::Immediately => {
                                    // Job is always going to the next queue
                                    maybe_queue = next_queue;
                                }
                                ScheduleCondition::LocalNaiveTimeWindow {
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
                                info!("job dropping off the pipeline: {job:?}");
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
                    info!(
                        "job dropped due to running out of error budget \
                         and no configured erroneous_jobs_queue: {job:?}"
                    );
                }
            }
            item.delete()?;
        }
        Ok((
            current_stop_start,
            num_jobs_handled,
            TerminationReason::QueueEmpty,
        ))
    }
}
