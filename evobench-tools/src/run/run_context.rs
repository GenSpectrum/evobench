use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, Local};

use crate::{
    date_and_time::time_ranges::DateTimeRange, info, serde_types::proper_filename::ProperFilename,
};

use super::{run_queue::RunQueue, run_queues::RunQueues, stop_start_status::StopStartStatus};

#[derive(Default)]
pub struct RunContext {
    stop_start_status: StopStartStatus,

    /// Queues with time ranges and flag `move_when_time_window_ends`,
    /// that were started processing entries from, with the end of the
    /// time range. To be closed off after the current time is past
    /// the end time.
    open_queues: BTreeMap<ProperFilename, DateTime<Local>>,
}

impl RunContext {
    pub fn stop_start_be(&mut self, stop_start: Option<&[String]>) -> Result<()> {
        self.stop_start_status.be(stop_start)
    }

    /// Notify the RunContext that this queue is being used (i.e. add
    /// it for later closing)
    pub fn running_job_in_windowed_queue(&mut self, queue: &RunQueue, dtr: DateTimeRange<Local>) {
        if queue.schedule_condition.move_when_time_window_ends() {
            self.open_queues.insert(queue.file_name.clone(), dtr.to);
        }
    }

    /// Check all open queues and run closing actions for those for
    /// which the time window has closed.
    // passing in RunQueues feels ugly; but want to have the queue
    // closing logic (handle_timeout) there.
    pub fn close_open_queues(
        &mut self,
        now: DateTime<Local>,
        run_queues: &RunQueues,
    ) -> Result<()> {
        let close: Vec<_> = self
            .open_queues
            .iter()
            .filter_map(|(file_name, dtr)| {
                if now >= *dtr {
                    Some(file_name.clone())
                } else {
                    None
                }
            })
            .collect();
        for file_name in close {
            if let Some(rqwn) = run_queues.get_run_queue_with_next_by_name(&file_name) {
                rqwn.handle_timeout()?;
                self.open_queues.remove(&file_name);
            } else {
                info!(
                    "couldn't find RunQueue with name {:?}, \
                     might have been config change",
                    file_name.as_str()
                )
            }
        }
        Ok(())
    }
}
