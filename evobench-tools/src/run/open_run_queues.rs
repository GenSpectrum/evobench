//! Does not belong in run_queues.rs? But must be accessible for utils
//! other than evobench.rs, too, thus this module.

use std::{thread, time::Duration};

use anyhow::Result;

use crate::{
    clone,
    run::{
        config::ShareableConfig, output_directory::index_files::regenerate_index_files,
        run_queues::RunQueues, sub_command::wd::open_queue_change_signals,
    },
    utillib::arc::CloneArc,
    warn,
};

pub fn open_run_queues(shareable_config: &ShareableConfig) -> Result<RunQueues> {
    // Get as argument? But there's really no harm done by multiple
    // mappings, OK?
    let mut signal_change = open_queue_change_signals(&shareable_config.global_app_state_dir)?;
    let signal_change_sender = signal_change.sender();

    thread::spawn({
        clone!(shareable_config);
        move || {
            loop {
                if signal_change.got_signals() {
                    if let Err(e) = regenerate_index_files(&shareable_config, None, None) {
                        // XX backoff
                        warn!("error: regenerate_index_files: {e:#}");
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
