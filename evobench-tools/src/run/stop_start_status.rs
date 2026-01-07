use anyhow::Result;

use crate::{debug, info, run::run_queue::run_command};

#[derive(Default)]
pub struct StopStartStatus {
    /// A running stop action, if any. To be finished (i.e. "start"
    /// sent to) on changes.
    current_stop_start: Option<Box<[String]>>,
}

impl StopStartStatus {
    /// Request the status to be the given `stop_start` argument;
    /// `None` means, no `stop_start` command is to be open, if given
    /// means the given command is to be open (i.e. between having
    /// been sent "stop" and "start").
    pub fn be(&mut self, stop_start: Option<&[String]>) -> Result<()> {
        if stop_start == self.current_stop_start.as_deref() {
            debug!("no change in stop_start command, leave it as is");
            Ok(())
        } else {
            if let Some(cmd) = &self.current_stop_start {
                info!("change in stop_start command: end the previous period");
                run_command(cmd.as_ref(), "start")?;
            }
            if let Some(cmd) = &stop_start {
                info!("change in stop_start command: begin a new period");
                run_command(cmd.as_ref(), "stop")?;
            }
            self.current_stop_start = stop_start.map(|l| l.into());
            Ok(())
        }
    }
}
