//! Does not belong in run_queues.rs? But must be accessible for utils
//! other than evobench.rs, too, thus this module.

use std::{thread, time::Duration};

use anyhow::Result;
use chj_unix_util::polling_signals::PollingSignals;

use crate::{
    clone,
    run::{
        config::ShareableConfig, output_directory::list::regenerate_list, run_queues::RunQueues,
    },
    utillib::arc::CloneArc,
    warn,
};

pub fn open_run_queues(shareable_config: &ShareableConfig) -> Result<RunQueues> {
    let run_queue_signal_change_path = shareable_config
        .global_app_state_dir
        .run_queue_signal_change_path();
    let mut signal_change = PollingSignals::open(&run_queue_signal_change_path, 0)?;
    let signal_change_sender = signal_change.sender();

    thread::spawn({
        clone!(shareable_config);
        move || {
            loop {
                if signal_change.got_signals() {
                    if let Err(e) = regenerate_list(&shareable_config, None, None) {
                        // XX backoff
                        warn!("error: regenerate_list: {e:#}");
                    }
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
    });

    RunQueues::open(
        shareable_config.run_config.queues.clone_arc(),
        true,
        &shareable_config.global_app_state_dir,
        Some(signal_change_sender.clone()),
    )
}
