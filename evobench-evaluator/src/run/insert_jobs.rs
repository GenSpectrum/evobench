use std::time::SystemTime;

use anyhow::{bail, Result};
use itertools::Itertools;

use crate::{
    key::{RunParameters, RunParametersHash},
    key_val_fs::key_val::{KeyVal, KeyValSync},
    serde::{date_and_time::system_time_to_rfc3339, git_url::GitUrl},
};

use super::{
    benchmarking_job::BenchmarkingJob, global_app_state_dir::GlobalAppStateDir,
    polling_pool::PollingPool, run_queues::RunQueues,
};

pub fn open_already_inserted(
    global_app_state_dir: &GlobalAppStateDir,
) -> Result<KeyVal<RunParametersHash, (RunParameters, Vec<SystemTime>)>> {
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
pub struct ForceAndQuiet {
    /// Normally, the same job parameters can only be inserted
    /// once, subsequent attempts yield an error. This overrides
    /// the check and allows insertion anyway.
    #[clap(long)]
    pub force: bool,

    /// Exit quietly if the given job parameters were already
    /// inserted before (by default, give an error)
    #[clap(long)]
    pub quiet: bool,
}

pub fn insert_jobs(
    benchmarking_jobs: Vec<BenchmarkingJob>,
    global_app_state_dir: &GlobalAppStateDir,
    remote_repository_url: &GitUrl,
    force_and_quiet: ForceAndQuiet,
    queues: &RunQueues,
) -> Result<()> {
    let ForceAndQuiet { force, quiet } = force_and_quiet;

    let already_inserted = open_already_inserted(&global_app_state_dir)?;
    let _lock = already_inserted.lock_exclusive()?;

    let mut polling_pool = PollingPool::open(
        remote_repository_url,
        &global_app_state_dir.working_directory_for_polling_pool_base()?,
    )?;

    for benchmarking_job in benchmarking_jobs {
        let run_parameters_hash = RunParametersHash::from(&benchmarking_job.run_parameters);

        let mut opt_entry = already_inserted.entry_opt(&run_parameters_hash)?;

        let insertion_times;
        if let Some(entry) = &mut opt_entry {
            let params;
            (params, insertion_times) = entry.get()?;
            if !force {
                if quiet {
                    return Ok(());
                } else {
                    let insertion_times = insertion_times
                        .iter()
                        .cloned()
                        .map(system_time_to_rfc3339)
                        .join(", ");
                    bail!(
                        "the parameters {params:?} were already inserted at: \
                             {insertion_times}"
                    )
                }
            }
        } else {
            insertion_times = Vec::new()
        }

        {
            let commit = &benchmarking_job.run_parameters.commit_id;
            if !polling_pool.commit_is_valid(commit)? {
                bail!("commit {commit} does not exist in the repository {remote_repository_url:?}")
            }
        }

        queues.first().push_front(&benchmarking_job)?;

        if let Some(mut entry) = opt_entry {
            entry.delete()?;
        }
        let mut insertion_times = insertion_times;
        insertion_times.push(SystemTime::now());
        already_inserted.insert(
            &run_parameters_hash,
            &(benchmarking_job.run_parameters.clone(), insertion_times),
            true,
        )?;
    }

    Ok(())
}
