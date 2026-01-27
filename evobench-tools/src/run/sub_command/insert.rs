use std::{path::PathBuf, str::FromStr};

use anyhow::{Result, anyhow};
use run_git::git::GitWorkingDir;

use crate::{
    config_file::backend_from_path,
    git::GitHash,
    key::RunParametersOpts,
    lazy::LazyResult,
    run::{
        benchmarking_job::{
            BenchmarkingJob, BenchmarkingJobOpts, BenchmarkingJobReasonOpt,
            BenchmarkingJobSettingsOpts,
        },
        config::RunConfigBundle,
        insert_jobs::{ForceOpt, QuietOpt, insert_jobs},
        run_queues::RunQueues,
    },
    serde::{git_branch_name::GitBranchName, priority::Priority},
    serde_util::serde_read_json,
};

#[derive(clap::Subcommand, Debug)]
pub enum Insert {
    /// Insert a job into the benchmarking queue. The given reference
    /// is resolved in a given working directory; if you have a commit
    /// id, then you can use the `insert` subcommand instead.
    InsertLocal {
        /// A Git reference to the commit that should be benchmarked
        /// (like `HEAD`, `master`, some commit id, etc.)
        reference: GitBranchName,

        /// The path to the Git working directory where `reference`
        /// should be resolved in
        #[clap(long, short, default_value = ".")]
        dir: PathBuf,

        #[clap(flatten)]
        benchmarking_job_settings: BenchmarkingJobSettingsOpts,
        #[clap(flatten)]
        reason: BenchmarkingJobReasonOpt,
        #[clap(flatten)]
        force_opt: ForceOpt,
        #[clap(flatten)]
        quiet_opt: QuietOpt,
    },

    /// Insert a job into the benchmarking queue, giving the commit id
    /// (hence, unlike the `insert-local` command, not requiring a
    /// working directory)
    Insert {
        #[clap(flatten)]
        benchmarking_job_opts: BenchmarkingJobOpts,

        #[clap(flatten)]
        force_opt: ForceOpt,
        #[clap(flatten)]
        quiet_opt: QuietOpt,
    },

    /// (Re-)insert a job from a job file
    InsertFile {
        #[clap(flatten)]
        benchmarking_job_settings_opts: BenchmarkingJobSettingsOpts,
        #[clap(flatten)]
        force_opt: ForceOpt,
        #[clap(flatten)]
        quiet_opt: QuietOpt,

        /// The initial priority boost, overrides the boost given in
        /// the file.
        #[clap(long)]
        initial_boost: Option<Priority>,

        /// Override the reason given in the file.
        #[clap(flatten)]
        reason: BenchmarkingJobReasonOpt,

        /// Path(s) to the JSON file(s) to insert. The format is the
        /// one used in the `~/.evobench-jobs/queues/` directories,
        /// except you can alternatively choose JSON5, RON, or one of
        /// the other formats shown in `config-formats` if the file
        /// has a corresponding file extension.
        paths: Vec<PathBuf>,
    },
}

impl Insert {
    pub fn run<F>(
        self,
        run_config_bundle: &RunConfigBundle,
        queues: &mut LazyResult<RunQueues, anyhow::Error, F>,
    ) -> Result<()>
    where
        F: FnOnce() -> Result<RunQueues, anyhow::Error>,
    {
        let conf = &run_config_bundle.run_config;

        match self {
            Insert::InsertLocal {
                reason,
                reference,
                dir,
                benchmarking_job_settings,
                force_opt,
                quiet_opt,
            } => {
                let git_working_dir = GitWorkingDir::from(dir);
                let commit_id_str = git_working_dir
                    .git_rev_parse(reference.as_str(), true)?
                    .ok_or_else(|| {
                        anyhow!("reference '{reference}' does not resolve to a commit")
                    })?;
                let commit_id = GitHash::from_str(&commit_id_str)?;

                let benchmarking_job_opts = BenchmarkingJobOpts {
                    reason,
                    benchmarking_job_settings,
                    run_parameters: RunParametersOpts { commit_id },
                };

                let queues = queues.force()?;
                insert_jobs(
                    benchmarking_job_opts.complete_jobs(
                        Some(&conf.benchmarking_job_settings),
                        &conf.job_templates_for_insert,
                    )?,
                    &run_config_bundle.global_app_state_dir,
                    &conf.remote_repository.url,
                    force_opt,
                    quiet_opt,
                    &queues,
                )?;
                Ok(())
            }

            Insert::Insert {
                benchmarking_job_opts,
                force_opt,
                quiet_opt,
            } => {
                let queues = queues.force()?;
                insert_jobs(
                    benchmarking_job_opts.complete_jobs(
                        Some(&conf.benchmarking_job_settings),
                        &conf.job_templates_for_insert,
                    )?,
                    &run_config_bundle.global_app_state_dir,
                    &conf.remote_repository.url,
                    force_opt,
                    quiet_opt,
                    &queues,
                )?;
                Ok(())
            }

            Insert::InsertFile {
                benchmarking_job_settings_opts,
                initial_boost,
                reason,
                force_opt,
                quiet_opt,
                paths,
            } => {
                let mut benchmarking_jobs = Vec::new();
                for path in &paths {
                    let mut job: BenchmarkingJob = if let Ok(backend) = backend_from_path(&path) {
                        backend.load_config_file(&path)?
                    } else {
                        serde_read_json(&path)?
                    };

                    job.check_and_init(
                        conf,
                        true,
                        &benchmarking_job_settings_opts,
                        initial_boost,
                        &reason,
                    )?;

                    benchmarking_jobs.push(job);
                }

                let queues = queues.force()?;
                let n = insert_jobs(
                    benchmarking_jobs,
                    &run_config_bundle.global_app_state_dir,
                    &conf.remote_repository.url,
                    force_opt,
                    quiet_opt,
                    &queues,
                )?;
                println!("Inserted {n} jobs.");
                Ok(())
            }
        }
    }
}
