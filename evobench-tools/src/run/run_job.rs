//! Running a benchmarking job

use std::{
    io::{Write, stderr},
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use chrono::{DateTime, Local};
use cj_path_util::path_util::{AppendToPath, rename_tmp_path};
use itertools::Itertools;
use nix::unistd::getpid;
use regex::Regex;

use crate::{
    config_file::ron_to_string_pretty,
    ctx,
    git::GitHash,
    git_tags::GitTags,
    info,
    io_utils::{
        capture::{CaptureOptions, OutFile},
        temporary_file::TemporaryFile,
    },
    key::{BenchmarkingJobParameters, RunParameters},
    run::{
        bench_tmp_dir::bench_tmp_dir,
        benchmarking_job::BenchmarkingJob,
        config::RunConfig,
        dataset_dir_env_var::dataset_dir_for,
        env_vars::assert_evobench_env_var,
        output_directory_structure::KeyDir,
        post_process::compress_file_as,
        run_queues::RunQueuesData,
        versioned_dataset_dir::VersionedDatasetDir,
        working_directory::{FetchTags, FetchedTags, WorkingDirectory},
    },
    serde::{date_and_time::DateTimeWithOffset, proper_dirname::ProperDirname},
    utillib::logging::{LogLevel, log_level},
};

use super::{
    config::{BenchmarkingCommand, ScheduleCondition},
    working_directory_pool::{WorkingDirectoryId, WorkingDirectoryPool},
};

/// Get the string for the `COMMIT_TAGS` env var, e.g. "" or
/// "foo,v1.2.3". Wants to be assured that `git fetch --tags` was run
/// (see methods that return a `FetchedTags`).
pub fn get_commit_tags(
    working_dir: &WorkingDirectory,
    commit_id: &GitHash,
    re: &Regex,
    fetched_tags: FetchedTags,
) -> Result<String> {
    if fetched_tags != FetchedTags::Yes {
        bail!("need up to date tags, but got {fetched_tags:?}")
    }

    let git_working_dir = &working_dir.git_working_dir;

    let git_tags = GitTags::from_dir(git_working_dir)?;
    // (Huh, `let s = ` being required here makes
    // no sense to me. rustc 1.90.0)
    let s = git_tags
        .get_by_commit(&commit_id)
        .filter(|s| re.is_match(s))
        .join(",");
    Ok(s)
}

/// The context for running a job (information that should not be part
/// of `Key`). Only used for one run, as some fields
/// (e.g. `timestamp`) change. Independent of jobs: this context is
/// bundled before selecting the job to run.
pub struct JobRunner<'pool> {
    pub working_directory_pool: &'pool mut WorkingDirectoryPool,
    pub output_base_dir: &'pool Path,
    /// The timestamp for this run.
    pub timestamp: DateTimeWithOffset,
    // Separate lifetime?
    pub run_config: &'pool RunConfig,
    // ditto?
    pub versioned_dataset_dir: &'pool VersionedDatasetDir,
}

impl<'pool> JobRunner<'pool> {
    pub fn timestamp_local(&self) -> DateTime<Local> {
        // (A little bit costly.)
        self.timestamp.to_datetime().into()
    }
}

pub struct JobRunnerJobData<'run_queues, 'j, 's> {
    pub job: &'j BenchmarkingJob,
    pub run_queues_data: &'s RunQueuesData<'run_queues>,
}

pub struct JobRunnerWithJob<'pool, 'run_queues, 'j, 's> {
    pub job_runner: JobRunner<'pool>,
    pub job_data: JobRunnerJobData<'run_queues, 'j, 's>,
}

impl<'run_queues, 'j, 's> JobRunnerJobData<'run_queues, 'j, 's> {
    /// Whether more job runs need to be done for the same commit, be
    /// it for the same job, or others.
    pub fn have_more_job_runs_for_same_commit(&self) -> bool {
        // Check if this the last run for the current job. `job` still
        // contains the count from before running it this time.
        if self.job.state.remaining_count > 1 {
            return true;
        }

        // Otherwise, look for *other* jobs than the current one. Not
        // so easy since jobs still don't contain an id? Except,
        // simply check if there is more than 1 entry.
        self.run_queues_data
            .jobs_by_commit_id(&self.job.public.run_parameters.commit_id)
            .len()
            > 1
    }
}

impl<'pool, 'run_queues, 'j, 's> JobRunnerWithJob<'pool, 'run_queues, 'j, 's> {
    pub fn run_job(
        &mut self,
        working_directory_id: WorkingDirectoryId,
        reason: &Option<String>,
        schedule_condition: &ScheduleCondition,
    ) -> Result<()> {
        // XX put that here, "for backwards compat", but could now use
        // something else for logging?
        let benchmarking_job_parameters = self.job_data.job.benchmarking_job_parameters();

        let BenchmarkingJobParameters {
            run_parameters,
            command,
        } = &benchmarking_job_parameters;
        let RunParameters {
            commit_id,
            custom_parameters,
        } = run_parameters.deref();

        let bench_tmp_dir = &bench_tmp_dir()?;
        info!(
            "bench_tmp_dir path, exists?: {:?}",
            (&bench_tmp_dir, bench_tmp_dir.exists())
        );

        // File for evobench library output
        let evobench_log;
        // File for other output, for optional use by target application
        let bench_output_log;
        {
            let pid = getpid();
            evobench_log = TemporaryFile::from(bench_tmp_dir.append(format!("evobench-{pid}.log")));
            bench_output_log =
                TemporaryFile::from(bench_tmp_dir.append(format!("bench-output-{pid}.log")));
        }

        // Remove any stale files from previous runs (we're not
        // removing all possible files (we leave files from other
        // processes alone (in case running multiple independent
        // daemons might be useful)), just those that would get in the
        // way).
        let _ = std::fs::remove_file(evobench_log.path());
        let _ = std::fs::remove_file(bench_output_log.path());

        let (opt_log_extraction, cleanup) = self
            .job_runner
            .working_directory_pool
            .process_in_working_directory(
                working_directory_id,
                &self.job_runner.timestamp,
                |mut working_directory| -> Result<Option<(&ProperDirname, PathBuf)>> {
                    // Have `checkout` always run git fetch to update
                    // the remote tags, to get them even if there have
                    // been past runs where they were not present yet;
                    // this is so that when the user changes the
                    // dataset directory and then makes a matching
                    // tag, we must have that matching tag. Also, for
                    // the release hack feature, we need to learn when
                    // the tag was added later. Thus always try to
                    // update from the git repository (failures
                    // leading to the working directory ending in
                    // error state and on repetition the job being
                    // aborted).
                    let fetched_tags = working_directory
                        .get()
                        .expect("not removed")
                        .checkout(commit_id.clone(), FetchTags::Always)?;

                    // Drop the lock on the pool
                    let working_directory = working_directory.into_inner().expect("not removed");

                    let dataset_dir = dataset_dir_for(
                        self.job_runner
                            .run_config
                            .versioned_datasets_base_dir
                            .as_deref(),
                        &custom_parameters,
                        self.job_runner.versioned_dataset_dir,
                        &working_directory.git_working_dir,
                        commit_id,
                        fetched_tags.clone(),
                    )?;

                    let commit_tags = get_commit_tags(
                        &working_directory,
                        &commit_id,
                        &self.job_runner.run_config.commit_tags_regex,
                        fetched_tags,
                    )?;

                    let BenchmarkingCommand {
                        target_name,
                        subdir,
                        command,
                        arguments,
                        pre_exec_bash_code,
                    } = command.deref();

                    let mut command = pre_exec_bash_code
                        .to_run_with_pre_exec(&self.job_runner.run_config)
                        .command(command, arguments);

                    let dir = working_directory
                        .git_working_dir
                        .working_dir_path_ref()
                        .append(subdir);

                    // for debugging info only:
                    let cmd_in_dir = format!("command {command:?} in directory {dir:?}");

                    info!(
                        "running {cmd_in_dir}, EVOBENCH_LOG={:?}...",
                        evobench_log.path()
                    );

                    let check = assert_evobench_env_var;

                    command
                        .envs(custom_parameters.btree_map())
                        .env(check("EVOBENCH_LOG"), evobench_log.path())
                        .env(check("BENCH_OUTPUT_LOG"), bench_output_log.path())
                        .env(check("COMMIT_ID"), commit_id.to_string())
                        .env(check("COMMIT_TAGS"), commit_tags)
                        .current_dir(&dir);
                    if let Some(dataset_dir) = &dataset_dir {
                        command.env(check("DATASET_DIR"), dataset_dir);
                    }

                    let command_output_file = OutFile::create(
                        &working_directory
                            .working_directory_path()
                            .standard_log_path(&self.job_runner.timestamp)?,
                    )?;

                    // Add info header in YAML -- XX abstraction, and
                    // move to / merge with `command_log_file.rs`?
                    command_output_file
                        .write_str(&serde_yml::to_string(&benchmarking_job_parameters)?)?;
                    command_output_file.write_str("\n")?;

                    info!(
                        "bench_tmp_dir path, exists?: {:?}",
                        (&bench_tmp_dir, bench_tmp_dir.exists())
                    );

                    let status = {
                        let mut other_files: Vec<Box<dyn Write + Send + 'static>> = vec![];
                        // Is it evil to use log_level() for this and not a
                        // function argument?
                        if log_level() >= LogLevel::Info {
                            other_files.push(Box::new(stderr()));
                        }
                        let other_files = Arc::new(Mutex::new(other_files));

                        command_output_file.run_with_capture(
                            command,
                            other_files,
                            CaptureOptions {
                                add_source_indicator: true,
                                add_timestamp: true,
                            },
                        )?
                    };

                    info!(
                        "bench_tmp_dir path, exists?: {:?}",
                        (&bench_tmp_dir, bench_tmp_dir.exists())
                    );

                    if status.success() {
                        info!("running {cmd_in_dir} succeeded");

                        Ok(Some((target_name, command_output_file.into_path())))
                    } else {
                        info!("running {cmd_in_dir} failed.");

                        info!(
                            "bench_tmp_dir path, exists?: {:?}",
                            (&bench_tmp_dir, bench_tmp_dir.exists())
                        );

                        let last_part = command_output_file.last_part(3000)?;
                        if log_level() < LogLevel::Info {
                            let mut err = stderr().lock();
                            writeln!(err, "---- run_job: error in dir {dir:?}: -------")?;
                            err.write_all(last_part.as_bytes())?;
                            writeln!(err, "---- /run_job: error in dir {dir:?} -------")?;
                        }

                        bail!(
                            "benchmarking command {cmd_in_dir} gave \
                             error status {status}, last_part {last_part:?}"
                        )
                    }
                },
                Some(&benchmarking_job_parameters),
                "run_job",
                Some(&|| self.job_data.have_more_job_runs_for_same_commit()),
            )?;

        // Can clean up right away, we're not currently accessing the
        // working directory any longer, OK?
        self.job_runner
            .working_directory_pool
            .working_directory_cleanup(cleanup)?;

        let log_extraction = if let Some(le) = opt_log_extraction {
            le
        } else {
            // It was dry run so must leave anyway, right? Todo:
            // this whole dry_run system is almost surely not
            // working any more and should probably be ripped out.
            return Ok(());
        };

        // The directory holding the full key information
        let key_dir = KeyDir::from_base_target_params(
            self.job_runner.output_base_dir,
            &command.target_name,
            &run_parameters,
        );

        // Below that, we make a dir for this particular run
        let run_dir = key_dir.append(&self.job_runner.timestamp)?;
        std::fs::create_dir_all(run_dir.path()).map_err(ctx!("create_dir_all {run_dir:?}"))?;

        info!("moving files to {run_dir:?}");

        // First try to compress the log file, here we check whether
        // it exists; before we expect to compress evobench.log
        // without checking its existence.
        if bench_output_log.path().exists() {
            compress_file_as(
                bench_output_log.path(),
                run_dir.bench_output_log_path(),
                false,
            )?;
            drop(bench_output_log);
        }

        let evobench_log_tmp =
            compress_file_as(evobench_log.path(), run_dir.evobench_log_path(), true)?;

        let (target_name, standard_log_tempfile) = log_extraction;
        compress_file_as(&standard_log_tempfile, run_dir.standard_log_path(), false)?;
        // It's OK to delete the original now, but we'll make use of
        // it for reading back.
        let standard_log_tempfile = TemporaryFile::from(standard_log_tempfile);

        {
            let target = run_dir.append_str("schedule_condition.ron")?;
            info!("saving context to {target:?}");
            let schedule_condition_str = ron_to_string_pretty(&schedule_condition)?;
            std::fs::write(&target, &schedule_condition_str)
                .map_err(ctx!("saving to {target:?}"))?
        }

        {
            let target = run_dir.append_str("reason.ron")?;
            info!("saving context to {target:?}");
            let s = ron_to_string_pretty(&reason)?;
            std::fs::write(&target, &s).map_err(ctx!("saving to {target:?}"))?
        }

        let evobench_log_path = evobench_log.path().to_owned();
        run_dir.post_process_single(
            Some(&evobench_log_path),
            move || {
                info!("evaluating the benchmark file succeeded");

                drop(evobench_log);

                rename_tmp_path(evobench_log_tmp)?;

                info!("compressed benchmark file renamed");
                Ok(())
            },
            // log extraction:
            target_name,
            standard_log_tempfile.path(),
            &self.job_runner.run_config,
            // Do not omit generation of evobench.log stats
            false,
        )?;

        key_dir.generate_summaries_for_key_dir(
            // Do not omit generation of evobench.log stats
            false,
        )?;

        Ok(())
    }
}
