//! Does not belong in run_queues.rs? But must be accessible for utils
//! other than evobench.rs, too, thus this module.

use std::{
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::Result;
use chj_unix_util::polling_signals::SharedPollingSignals;

use crate::{
    clone,
    run::{
        config::ShareableConfig, output_directory::index_files::regenerate_index_files,
        run_queues::RunQueues, sub_command::wd::open_queue_change_signals,
    },
    utillib::arc::CloneArc,
    warn,
};

#[must_use]
pub struct RegenerateIndexFiles {
    signal_change: SharedPollingSignals,
    shareable_config: ShareableConfig,
}

impl RegenerateIndexFiles {
    pub fn run_one(&self) {
        if let Some(signal) = self.signal_change.get_latest_signal() {
            if let Err(e) = regenerate_index_files(&self.shareable_config, None, None) {
                signal.ignore();
                // XX backoff
                warn!("error: regenerate_index_files: {e:#}");
            } else {
                signal.confirm();
            }
        }
    }

    pub fn spawn_runner_thread(self) -> std::io::Result<JoinHandle<()>> {
        thread::Builder::new()
            .name("regen-index-files".into())
            .spawn({
                move || {
                    loop {
                        self.run_one();
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            })
    }
}

pub fn open_run_queues(
    shareable_config: &ShareableConfig,
) -> Result<(RunQueues, RegenerateIndexFiles)> {
    // Get `signal_change` as argument? But there's really no harm
    // done by multiple mappings, OK?
    let signal_change = open_queue_change_signals(&shareable_config.global_app_state_dir)?;
    let signal_change_sender = signal_change.sender();
    clone!(shareable_config);
    Ok((
        RunQueues::open(
            shareable_config.run_config.queues.clone_arc(),
            true,
            &shareable_config.global_app_state_dir,
            Some(signal_change_sender),
        )?,
        RegenerateIndexFiles {
            signal_change,
            shareable_config,
        },
    ))
}
