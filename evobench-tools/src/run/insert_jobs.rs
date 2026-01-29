use std::{io::stdout, time::SystemTime};

use anyhow::{Result, bail};
use itertools::Itertools;

use crate::{
    config_file::ron_to_string_pretty,
    key::{BenchmarkingJobParameters, BenchmarkingJobParametersHash},
    key_val_fs::key_val::{KeyVal, KeyValSync},
    run::{config::RunConfigBundle, sub_command::open_polling_pool},
    serde::date_and_time::system_time_to_rfc3339,
};

use super::{
    benchmarking_job::BenchmarkingJob, global_app_state_dir::GlobalAppStateDir,
    run_queues::RunQueues,
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

#[derive(Debug, Clone, clap::Args)]
pub struct ForceOpt {
    /// Normally, the same job parameters can only be inserted
    /// once, subsequent attempts yield an error. This overrides
    /// the check and allows insertion anyway.
    #[clap(long)]
    pub force: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct QuietOpt {
    /// Skip attempts at insertion quietly if the given job parameters
    /// were already inserted before (by default, give an error)
    #[clap(long)]
    pub quiet: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct DryRunOpt {
    /// Show the details of the jobs that would be inserted
    /// instead of inserting them.
    #[clap(long)]
    dry_run: bool,
}

/// Unless `dry_run` is true (in which a report is printed to stdout),
/// inserts the jobs into the first queue and the `already_inserted`
/// table. Returns the number of jobs actually inserted. If a job has
/// already been inserted in the past and `force_opt` and `quiet_opt`
/// are both false, no job is inserted and an error listing all the
/// already-inserted jobs is returned instead. Also, if the commit id
/// of any job is not present in upstream, returns an error without
/// inserting any jobs.
pub fn insert_jobs(
    benchmarking_jobs: Vec<BenchmarkingJob>,
    config: &RunConfigBundle,
    dry_run_opt: DryRunOpt,
    force_opt: ForceOpt,
    quiet_opt: QuietOpt,
    queues: &RunQueues,
) -> Result<usize> {
    let DryRunOpt { dry_run } = dry_run_opt;
    let ForceOpt { force } = force_opt;
    let QuietOpt { quiet } = quiet_opt;

    let already_inserted = open_already_inserted(&config.global_app_state_dir)?;
    let _lock = already_inserted.lock_exclusive()?;

    let mut polling_pool = open_polling_pool(config)?;

    let mut jobs_to_insert: Vec<(
        BenchmarkingJob,
        BenchmarkingJobParametersHash,
        Vec<SystemTime>,
    )> = Vec::new();
    // Only if !quiet
    let mut jobs_already_inserted: Vec<String> = Vec::new();

    for benchmarking_job in benchmarking_jobs {
        let run_parameters_hash =
            BenchmarkingJobParametersHash::from(&benchmarking_job.benchmarking_job_parameters());

        // All insertion times, for adding the new ones below
        let insertion_times;

        // Check if already inserted
        if let Some(mut entry) = already_inserted.entry_opt(&run_parameters_hash)? {
            let params;
            (params, insertion_times) = entry.get()?;
            if force {
                // fall through and try to do insertion anyway, below
            } else {
                if !quiet {
                    let insertion_times = insertion_times
                        .iter()
                        .cloned()
                        .map(|t| system_time_to_rfc3339(t, true))
                        .join(", ");
                    jobs_already_inserted.push(format!(
                        "These parameters have already been inserted at {insertion_times}:\n{}",
                        ron_to_string_pretty(&params).expect("no err")
                    ));
                }
                // skip insertion
                continue;
            }
        } else {
            insertion_times = Vec::new()
        }

        {
            let commit = &benchmarking_job.public.run_parameters.commit_id;
            if !polling_pool.commit_is_valid(commit)? {
                bail!(
                    "commit {commit} does not exist in the repository at {:?}",
                    config.run_config.remote_repository.url.as_str()
                )
            }
        }
        // ^ XX so, always checks upstream repo for the commit! hm OK sure?

        jobs_to_insert.push((benchmarking_job, run_parameters_hash, insertion_times));
    }

    if !jobs_already_inserted.is_empty() {
        bail!(
            "there are jobs that were already inserted:\n{}",
            jobs_already_inserted.join("\n\n")
        )
    }

    if dry_run {
        use std::io::Write;
        let mut out = stdout().lock();
        for (i, (benchmarking_job, _run_parameters_hash, _insertion_times)) in
            jobs_to_insert.into_iter().enumerate()
        {
            writeln!(
                &mut out,
                "would insert job {}:\n{}",
                i + 1,
                ron_to_string_pretty(&benchmarking_job).expect("no err")
            )?;
        }
        out.flush()?;
        Ok(0)
    } else {
        // Insert the jobs
        let mut num_inserted = 0;
        for (benchmarking_job, run_parameters_hash, mut insertion_times) in jobs_to_insert {
            queues.first().push_front(&benchmarking_job)?;
            num_inserted += 1;

            // Update the `already_inserted` table (have to re-request
            // entry as it contains a file handle (and lock?, but even
            // without a lock it's bad)) -- XXX do we get a lock on the whole table, though?
            if let Some(entry) = already_inserted.entry_opt(&run_parameters_hash)? {
                entry.delete()?;
            }
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
}
