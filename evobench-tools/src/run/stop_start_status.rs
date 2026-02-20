use anyhow::{Result, bail};
use std::process::Command;

use crate::{ctx, debug, info};

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
