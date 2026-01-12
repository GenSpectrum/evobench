use std::time::SystemTime;

use anyhow::{Result, bail};
use itertools::Itertools;

use crate::{
    config_file::ron_to_string_pretty,
    key::{BenchmarkingJobParameters, BenchmarkingJobParametersHash},
    key_val_fs::key_val::{KeyVal, KeyValSync},
    serde::{date_and_time::system_time_to_rfc3339, git_url::GitUrl},
};

use super::{
    benchmarking_job::BenchmarkingJob, global_app_state_dir::GlobalAppStateDir,
    polling_pool::PollingPool, run_queues::RunQueues,
};

pub fn open_already_inserted(
    global_app_state_dir: &GlobalAppStateDir,
) -> Result<KeyVal<BenchmarkingJobParametersHash, (BenchmarkingJobParameters, Vec<SystemTime>)>> {
    Ok(KeyVal::open(
        global_app_state_dir.already_inserted_base()?,
        crate::key_val_fs::key_val::KeyValConfig {
            sync: KeyValSync::All,
            // already created anyway
            create_dir_if_not_exists: false,
        },
    )?)
}

#[derive(Debug, clap::Args)]
pub struct ForceOpt {
    /// Normally, the same job parameters can only be inserted
    /// once, subsequent attempts yield an error. This overrides
    /// the check and allows insertion anyway.
    #[clap(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct QuietOpt {
    /// Skip attempts at insertion quietly if the given job parameters
    /// were already inserted before (by default, give an error)
    #[clap(long)]
    pub quiet: bool,
}

/// Returns the number of jobs actually inserted
pub fn insert_jobs(
    benchmarking_jobs: Vec<BenchmarkingJob>,
    global_app_state_dir: &GlobalAppStateDir,
    remote_repository_url: &GitUrl,
    force_opt: ForceOpt,
    quiet_opt: QuietOpt,
    queues: &RunQueues,
) -> Result<usize> {
    let ForceOpt { force } = force_opt;
    let QuietOpt { quiet } = quiet_opt;

    let already_inserted = open_already_inserted(&global_app_state_dir)?;
    let _lock = already_inserted.lock_exclusive()?;

    let mut polling_pool = PollingPool::open(
        remote_repository_url,
        &global_app_state_dir.working_directory_for_polling_pool_base()?,
    )?;

    let mut num_inserted = 0;

    for benchmarking_job in benchmarking_jobs {
        let run_parameters_hash =
            BenchmarkingJobParametersHash::from(&benchmarking_job.benchmarking_job_parameters());

        // All insertion times, for adding the new ones below
        let insertion_times;

        // Check if already inserted
        let mut opt_entry = already_inserted.entry_opt(&run_parameters_hash)?;
        if let Some(entry) = &mut opt_entry {
            let params;
            (params, insertion_times) = entry.get()?;
            if force {
                // fall through and do insertion anyway, below
            } else {
                if quiet {
                    // skip insertion
                    continue;
                } else {
                    let insertion_times = insertion_times
                        .iter()
                        .cloned()
                        .map(|t| system_time_to_rfc3339(t, true))
                        .join(", ");
                    bail!(
                        "the parameters {} have already been inserted at {insertion_times}",
                        ron_to_string_pretty(&params).expect("no err")
                    )
                }
            }
        } else {
            insertion_times = Vec::new()
        }

        {
            let commit = &benchmarking_job
                .benchmarking_job_public
                .run_parameters
                .commit_id;
            if !polling_pool.commit_is_valid(commit)? {
                bail!(
                    "commit {commit} does not exist in the repository at {:?}",
                    remote_repository_url.as_str()
                )
            }
        }

        // insert it
        queues.first().push_front(&benchmarking_job)?;
        num_inserted += 1;

        // update the `already_inserted` table
        if let Some(entry) = opt_entry {
            entry.delete()?;
        }
        let mut insertion_times = insertion_times;
        insertion_times.push(SystemTime::now());
        already_inserted.insert(
            &run_parameters_hash,
            &(
                benchmarking_job.benchmarking_job_parameters(),
                insertion_times,
            ),
            true,
        )?;
    }

    Ok(num_inserted)
}
