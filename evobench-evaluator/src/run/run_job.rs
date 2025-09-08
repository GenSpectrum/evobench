//! Running a benchmarking job

use std::{
    io::{stderr, Write},
    ops::Deref,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use nix::{unistd::getpid, unistd::getuid};
use run_git::path_util::{add_extension, AppendToPath};
use strum_macros::EnumString;

use crate::{
    config_file::ron_to_string_pretty,
    ctx, info,
    io_utils::{
        bash::bash_string_from_program_and_args,
        capture::{CaptureOptions, OutFile},
        temporary_file::TemporaryFile,
    },
    key::{BenchmarkingJobParameters, RunParameters},
    path_util::rename_tmp_path,
    run::{
        benchmarking_job::BenchmarkingJob, config::RunConfig, output_directory_structure::KeyDir,
        run_queues::RunQueuesData,
    },
    serde::{
        allowed_env_var::AllowEnvVar, date_and_time::DateTimeWithOffset,
        proper_dirname::ProperDirname,
    },
    utillib::logging::{log_level, LogLevel},
    zstd_file::compress_file,
};

use super::{
    config::{BenchmarkingCommand, ScheduleCondition},
    working_directory_pool::{WorkingDirectoryId, WorkingDirectoryPool},
};

// ------------------------------------------------------------------
pub const EVOBENCH_ENV_VARS: &[&str] = &["EVOBENCH_LOG", "BENCH_OUTPUT_LOG", "COMMIT_ID"];

pub fn is_evobench_env_var(s: &str) -> bool {
    EVOBENCH_ENV_VARS.contains(&s)
}

/// A parameter for `AllowedEnvVar` that checks that the variable is
/// not going to conflict with one of the built-in evobench env vars
/// (in the future perhaps also check for things like USER?)
#[derive(Debug)]
pub struct AllowableCustomEnvVar;
impl AllowEnvVar for AllowableCustomEnvVar {
    const MAX_ENV_VAR_NAME_LEN: usize = 80;

    fn allow_env_var(s: &str) -> bool {
        !is_evobench_env_var(s)
    }

    fn expecting() -> String {
        format!(
            "a variable name that is *not* any of {}",
            EVOBENCH_ENV_VARS.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::serde::allowed_env_var::AllowedEnvVar;

    use super::*;

    #[test]
    fn t_allowable_custom_env_var_name() {
        let allow = AllowableCustomEnvVar::allow_env_var;
        assert!(allow("FOO"));
        // We don't care whether the user decides to use unorthodox
        // variable names
        assert!(allow("foo"));
        assert!(allow("%&/',é\nhmm"));
        assert!(allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));
        // Too long, but have to rely on `AllowedEnvVar` to get the
        // `MAX_ENV_VAR_NAME_LEN` constant checked
        assert!(allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));

        let allow =
            |s: &str| -> bool { AllowedEnvVar::<AllowableCustomEnvVar>::from_str(s).is_ok() };

        assert!(allow("foo"));
        assert!(allow("%&/',é\nhmm"));
        assert!(allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));

        // Problems caughtby `AllowedEnvVar::from_str`
        assert!(!allow(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        ));
        assert!(!allow("A\0B"));
        assert!(!allow("foo=bar"));
        assert!(!allow("EVOBENCH_LOG"));

        assert_eq!(
            AllowedEnvVar::<AllowableCustomEnvVar>::from_str("EVOBENCH_LOG")
                .err()
                .unwrap()
                .to_string(),
            "AllowableCustomEnvVar env variable \"EVOBENCH_LOG\" is reserved, expecting a variable name \
             that is *not* any of EVOBENCH_LOG, BENCH_OUTPUT_LOG, COMMIT_ID"
        );
    }
}

// Can't make this const easily, but doesn't matter. It'll catch bugs
// on the first job run.
fn assert_evobench_env_var(s: &str) -> &str {
    if is_evobench_env_var(s) {
        s
    } else {
        panic!("Not a known EVOBENCH_ENV_VARS entry: {s:?}")
    }
}
// ------------------------------------------------------------------

#[derive(Debug, EnumString, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum DryRun {
    DoNothing,
    DoWorkingDir,
    DoAll,
}

impl DryRun {
    fn means(self, done: Self) -> bool {
        self as u8 <= done as u8
    }
}

// I am tired
fn get_username() -> Result<String> {
    std::env::var("USER").map_err(ctx!("can't get USER environment variable"))
}

/// Returns the path to a temporary directory, creating it if
/// necessary and checking ownership if it already exists. The
/// directory is not unique for all processes, but shared for all
/// evobench-run instances--which is OK both because we only do 1 run
/// at the same time (and take a lock to ensure that), but also
/// because we're now currently actually also adding the pid to the
/// file paths inside.
fn bench_tmp_dir() -> Result<PathBuf> {
    // XX use src/installation/binaries_repo.rs from xmlhub-indexer
    // instead once that's separated?
    let user = get_username()?;
    match std::env::consts::OS {
        "linux" => {
            let tmp: PathBuf = format!("/dev/shm/{user}").into();
            match std::fs::create_dir(&tmp) {
                Ok(()) => Ok(tmp),
                Err(e) => match e.kind() {
                    std::io::ErrorKind::AlreadyExists => {
                        let m = std::fs::metadata(&tmp)?;
                        let dir_uid = m.uid();
                        let uid: u32 = getuid().into();
                        if dir_uid == uid {
                            Ok(tmp)
                        } else {
                            bail!(
                                "bench_tmp_dir: directory {tmp:?} should be owned by \
                                 the user {user:?} which is set in the USER env var, \
                                 but the uid owning that directory is {dir_uid} whereas \
                                 the current process is running as {uid}"
                            )
                        }
                    }
                    _ => Err(e).map_err(ctx!("create_dir {tmp:?}")),
                },
            }
        }
        _ => {
            let tmp: PathBuf = "./tmp".into();
            std::fs::create_dir_all(&tmp).map_err(ctx!("create_dir_all {tmp:?}"))?;
            Ok(tmp)
        }
    }
}

/// The context for running a job (information that should not be part
/// of `Key`). Only used for one run, as some fields
/// (e.g. `timestamp`) change. Independent of jobs: this context is
/// bundled before selecting the job to run.
pub struct JobRunner<'pool> {
    pub working_directory_pool: &'pool mut WorkingDirectoryPool,
    pub output_base_dir: &'pool Path,
    pub dry_run: DryRun,
    /// The timestamp for this run.
    pub timestamp: DateTimeWithOffset,
    // Separate lifetime?
    pub run_config: &'pool RunConfig,
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
        if self.job.benchmarking_job_state.remaining_count > 1 {
            return true;
        }

        // Otherwise, look for *other* jobs than the current one. Not
        // so easy since jobs still don't contain an id? Except,
        // simply check if there is more than 1 entry.
        self.run_queues_data
            .jobs_by_commit_id(&self.job.benchmarking_job_public.run_parameters.commit_id)
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

        if self.job_runner.dry_run.means(DryRun::DoNothing) {
            println!("dry-run: would run {benchmarking_job_parameters:?}");
            return Ok(());
        }
        let BenchmarkingJobParameters {
            run_parameters,
            command,
        } = &benchmarking_job_parameters;
        let RunParameters {
            commit_id,
            custom_parameters,
        } = run_parameters.deref();

        let bench_tmp_dir = bench_tmp_dir()?;

        let pid = getpid();
        // File for evobench library output
        let evobench_log =
            TemporaryFile::from((&bench_tmp_dir).append(format!("evobench-{pid}.log")));
        // File for other output, for optional use by target application
        let bench_output_log =
            TemporaryFile::from((&bench_tmp_dir).append(format!("bench-output-{pid}.log")));

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
                |working_directory| -> Result<Option<(&ProperDirname, PathBuf)>> {
                    working_directory.checkout(commit_id.clone())?;

                    if self.job_runner.dry_run.means(DryRun::DoWorkingDir) {
                        println!("checked out working directory: {working_directory_id}");
                        return Ok(None);
                    }

                    let BenchmarkingCommand {
                        target_name,
                        subdir,
                        command,
                        arguments,
                    } = command.deref();

                    let dir = working_directory
                        .git_working_dir
                        .working_dir_path_ref()
                        .append(subdir);

                    // for debugging info only:
                    let cmd_in_dir = {
                        let cmd =
                            bash_string_from_program_and_args(command.to_string_lossy(), arguments);
                        format!("command {cmd:?} in directory {dir:?}")
                    };

                    info!(
                        "running {cmd_in_dir}, EVOBENCH_LOG={:?}...",
                        evobench_log.path()
                    );

                    let mut command = Command::new(command);
                    let check = assert_evobench_env_var;
                    command
                        .envs(custom_parameters.btree_map())
                        .env(check("EVOBENCH_LOG"), evobench_log.path())
                        .env(check("BENCH_OUTPUT_LOG"), bench_output_log.path())
                        .env(check("COMMIT_ID"), commit_id.to_string())
                        .args(arguments)
                        .current_dir(&dir);

                    let command_output_file = OutFile::create(
                        &add_extension(
                            working_directory.git_working_dir.working_dir_path_ref(),
                            format!(
                                "output_of_benchmarking_command_at_{}",
                                self.job_runner.timestamp
                            ),
                        )
                        .expect("has filename"),
                    )?;

                    // Add info header in YAML -- XX abstraction, and
                    // move to / merge with `command_log_file.rs`?
                    command_output_file
                        .write_str(&serde_yml::to_string(&benchmarking_job_parameters)?)?;
                    command_output_file.write_str("\n")?;

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

                    if status.success() {
                        info!("running {cmd_in_dir} succeeded");

                        Ok(Some((target_name, command_output_file.into_path())))
                    } else {
                        info!("running {cmd_in_dir} failed.");
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

        let compress_file_as = |source_file: &TemporaryFile,
                                target_filename: &str,
                                add_tmp: bool|
         -> Result<PathBuf> {
            let target_filename = add_extension(target_filename, "zstd").expect("got filename");
            let target_filename = if add_tmp {
                add_extension(target_filename, "tmp").expect("got filename")
            } else {
                target_filename
            };
            let target = run_dir.append_str(&target_filename.to_string_lossy())?;
            compress_file(
                source_file.path(),
                &target,
                // be quiet when:
                log_level() < LogLevel::Info,
            )?;
            // Do *not* remove the source file here as
            // TemporaryFile::drop will do it.
            Ok(target)
        };

        // First try to compress the log file, here we check whether
        // it exists; before we expect to compress evobench.log
        // without checking its existence.
        if bench_output_log.path().exists() {
            compress_file_as(&bench_output_log, "bench_output.log", false)?;
            drop(bench_output_log);
        }
        let evobench_log_tmp = compress_file_as(&evobench_log, "evobench.log", true)?;

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
            opt_log_extraction,
            &self.job_runner.run_config,
        )?;

        key_dir.generate_summaries_for_key_dir()?;

        Ok(())
    }
}
