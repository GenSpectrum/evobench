//! Running a benchmarking job

use std::{
    collections::{hash_map::Entry, HashMap},
    ffi::OsString,
    io::{stderr, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Result};
use nix::libc::{getpid, getuid};
use run_git::path_util::{add_extension, AppendToPath};
use strum_macros::EnumString;

use crate::{
    config_file::ron_to_string_pretty,
    ctx, info,
    io_utils::{
        capture::{CaptureOpts, OutFile},
        temporary_file::TemporaryFile,
    },
    key::RunParameters,
    serde::proper_filename::ProperFilename,
    utillib::logging::{log_level, LogLevel},
    zstd_file::compress_file,
};

use super::{
    allowed_env_var::AllowEnvVar,
    config::{BenchmarkingCommand, ScheduleCondition},
    working_directory_pool::WorkingDirectoryPool,
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

/// 'Temporary' directory, it's OK to only have one for all since we
/// have a lock. Creates it already.
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
                        let uid = unsafe { getuid() }; // XX why is this unsafe?
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

// XX here, *too*, do capture for consistency? XX: could do "nice" scheduling here.
fn evobench_evaluator(args: &[OsString]) -> Result<()> {
    let prog = "evobench-evaluator";
    let mut c = Command::new(prog);
    c.args(args);
    let mut child = c.spawn().map_err(ctx!("spawning command {c:?}"))?;
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        bail!("running {prog:?} with args {args:?}: {status}")
    }
}

pub fn run_job(
    working_directory_pool: &mut WorkingDirectoryPool,
    reason: &Option<String>,
    checked_run_parameters: &RunParameters,
    schedule_condition: &ScheduleCondition,
    dry_run: DryRun,
    benchmarking_command: &BenchmarkingCommand,
    output_base_dir: &Path,
) -> Result<()> {
    if dry_run.means(DryRun::DoNothing) {
        println!("dry-run: would run {checked_run_parameters:?}");
        return Ok(());
    }
    let RunParameters {
        commit_id,
        custom_parameters,
    } = checked_run_parameters;

    let working_directory_id =
        working_directory_pool.get_a_working_directory_for_commit(&commit_id)?;

    // Errors after finishing the benchmarking (post-processing phase)
    // are passed in an Option in `post_benchmark_error`, to avoid the
    // directory being marked as erroneous.
    let post_benchmark_error = working_directory_pool.process_working_directory(
        working_directory_id,
        |working_directory, timestamp| -> Result<Option<anyhow::Error>> {
            working_directory.checkout(commit_id.clone())?;

            if dry_run.means(DryRun::DoWorkingDir) {
                println!("checked out working directory: {working_directory_id:?}");
                return Ok(None);
            }

            let BenchmarkingCommand {
                subdir,
                command,
                arguments,
            } = benchmarking_command;

            let dir = working_directory
                .git_working_dir
                .working_dir_path_ref()
                .append(subdir);

            let bench_tmp_dir = bench_tmp_dir()?;

            let pid = unsafe { getpid() };
            // File for evobench library output
            let evobench_log =
                TemporaryFile::from((&bench_tmp_dir).append(format!("evobench-{pid}.log")));
            // File for other output, for optional use by target application
            let bench_output_log =
                TemporaryFile::from((&bench_tmp_dir).append(format!("bench-output-{pid}.log")));

            let _ = std::fs::remove_file(evobench_log.path());

            // for debugging info only:
            let cmd_in_dir = {
                let mut cmd = vec![command.to_string_lossy().into_owned()];
                cmd.append(&mut arguments.clone());
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

            let command_output_file_path = add_extension(
                working_directory.git_working_dir.working_dir_path_ref(),
                format!("output_of_benchmarking_command_at_{timestamp}"),
            )
            .expect("has filename");
            let command_output_file = OutFile::create(&command_output_file_path)?;
            {
                // Add info header
                command_output_file.write_str(&serde_yml::to_string(checked_run_parameters)?)?;
            }

            let mut other_files: Vec<Box<dyn Write + Send + 'static>> = vec![];
            // Is it evil to use log_level() for this and not a
            // function argument?
            if log_level() >= LogLevel::Info {
                other_files.push(Box::new(stderr()));
            }
            let other_files = Arc::new(Mutex::new(other_files));

            let status = command_output_file.run_with_capture(
                command,
                other_files,
                CaptureOpts {
                    add_source_indicator: true,
                    add_timestamp: true,
                },
            )?;

            if status.success() {
                info!("running {cmd_in_dir} succeeded");

                // The directory holding the full key information
                let key_dir = checked_run_parameters.extend_path(output_base_dir.to_owned());
                // Below that, we make a dir for this particular run
                let result_dir = (&key_dir).append(timestamp.as_str());
                std::fs::create_dir_all(&result_dir)
                    .map_err(ctx!("create_dir_all {result_dir:?}"))?;

                info!("moving files to {result_dir:?}");

                let compress_file_as =
                    |source_file: &TemporaryFile, target_filename: &str| -> Result<()> {
                        let target_filename =
                            add_extension(target_filename, "zstd").expect("got filename");
                        let target = (&result_dir).append(target_filename);
                        compress_file(
                            source_file.path(),
                            &target,
                            // be quiet when:
                            log_level() < LogLevel::Info,
                        )?;
                        // Do *not* remove the source file here as
                        // TemporaryFile::drop will do it.
                        Ok(())
                    };
                compress_file_as(&evobench_log, "evobench.log")?;
                compress_file_as(&bench_output_log, "bench_output.log")?;

                info!("evaluating benchmark file");

                // Doing this *before* moving the files, as a way to
                // ensure that no invalid files end up in the results
                // pool!

                evobench_evaluator(&vec![
                    "single".into(),
                    evobench_log.path().into(),
                    "--show-thread-number".into(),
                    "--excel".into(),
                    (&result_dir).append("single.xlsx").into(),
                ])?;

                // It's a bit inefficient to read the $EVOBENCH_LOG
                // twice, but currently can't change the options
                // (--show-thread-number) without a separate run, also
                // the cost is just a second or so.
                evobench_evaluator(&vec![
                    "single".into(),
                    evobench_log.path().into(),
                    "--flame".into(),
                    (&result_dir).append("single").into(),
                ])?;

                info!("evaluating the benchmark file succeeded");

                {
                    // HACK to allow for the SILO
                    // benchmarking/Makefile to move away the
                    // EVOBENCH_LOG file after preprocessing, and have
                    // that archived here. For summaries, have to run
                    // the evobench-evaluator on those manually,
                    // though.
                    let evobench_log_preprocessing = TemporaryFile::from(
                        (&bench_tmp_dir).append(format!("evobench-{pid}.log-preprocessing.log")),
                    );
                    if evobench_log_preprocessing.path().exists() {
                        compress_file_as(
                            &evobench_log_preprocessing,
                            "evobench-preprocessing.log",
                        )?;

                        evobench_evaluator(&vec![
                            "single".into(),
                            evobench_log_preprocessing.path().into(),
                            "--show-thread-number".into(),
                            "--excel".into(),
                            (&result_dir).append("single-preprocessing.xlsx").into(),
                        ])?;
                        evobench_evaluator(&vec![
                            "single".into(),
                            evobench_log_preprocessing.path().into(),
                            "--flame".into(),
                            (&result_dir).append("single-preprocessing").into(),
                        ])?;
                    }
                }

                {
                    let target = (&result_dir).append("schedule_condition.ron");
                    info!("saving context to {target:?}");
                    let schedule_condition_str = ron_to_string_pretty(&schedule_condition)?;
                    std::fs::write(&target, &schedule_condition_str)
                        .map_err(ctx!("saving to {target:?}"))?
                }

                {
                    let target = (&result_dir).append("reason.ron");
                    info!("saving context to {target:?}");
                    let s = ron_to_string_pretty(&reason)?;
                    std::fs::write(&target, &s).map_err(ctx!("saving to {target:?}"))?
                }

                info!("(re-)evaluating the summary file across all results for this key");

                let res = (|| -> Result<()> {
                    fn generate_summary<P: AsRef<Path>>(
                        key_dir: &PathBuf,
                        job_output_dirs: &[P],
                        target_type_opt: &str,
                        file_base_name: &str,
                    ) -> Result<()> {
                        let mut args: Vec<OsString> = vec!["summary".into()];
                        args.push(target_type_opt.into());
                        args.push(key_dir.append(file_base_name).into());

                        for job_output_dir in job_output_dirs {
                            let evobench_log = job_output_dir.as_ref().append("evobench.log.zstd");
                            if std::fs::exists(&evobench_log)
                                .map_err(ctx!("checking path {evobench_log:?}"))?
                            {
                                args.push(evobench_log.into());
                            } else {
                                info!("missing file {evobench_log:?}, empty dir?");
                            }
                        }

                        evobench_evaluator(&args)?;

                        Ok(())
                    }

                    let job_output_dirs: Vec<PathBuf> = std::fs::read_dir(&key_dir)
                        .map_err(ctx!("opening dir {key_dir:?}"))?
                        .map(|entry| -> Result<Option<PathBuf>, std::io::Error> {
                            let entry: std::fs::DirEntry = entry?;
                            let ft = entry.file_type()?;
                            if ft.is_dir() {
                                Ok(Some(entry.path()))
                            } else {
                                Ok(None)
                            }
                        })
                        .filter_map(|r| r.transpose())
                        .collect::<Result<_, _>>()
                        .map_err(ctx!("getting dir listing for {key_dir:?}"))?;

                    generate_summary(&key_dir, &job_output_dirs, "--excel", "summary.xlsx")?;
                    generate_summary(&key_dir, &job_output_dirs, "--flame", "summary")?;

                    let mut job_output_dirs_by_situation: HashMap<ProperFilename, Vec<&PathBuf>> =
                        HashMap::new();
                    for job_output_dir in &job_output_dirs {
                        let schedule_condition_path =
                            job_output_dir.append("schedule_condition.ron");
                        match std::fs::read_to_string(&schedule_condition_path) {
                            Ok(s) => {
                                let schedule_condition: ScheduleCondition = ron::from_str(&s)
                                    .map_err(ctx!("reading file {schedule_condition_path:?}"))?;
                                if let Some(situation) = schedule_condition.situation() {
                                    // XX it's just too long, proper abstraction pls?
                                    match job_output_dirs_by_situation.entry(situation.clone()) {
                                        Entry::Occupied(mut occupied_entry) => {
                                            occupied_entry.get_mut().push(job_output_dir);
                                        }
                                        Entry::Vacant(vacant_entry) => {
                                            vacant_entry.insert(vec![job_output_dir]);
                                        }
                                    }
                                }
                            }
                            Err(e) => match e.kind() {
                                std::io::ErrorKind::NotFound => (),
                                _ => Err(e)
                                    .map_err(ctx!("reading file {schedule_condition_path:?}"))?,
                            },
                        }
                    }

                    for (situation, job_output_dirs) in job_output_dirs_by_situation.iter() {
                        generate_summary(
                            &key_dir,
                            job_output_dirs.as_slice(),
                            "--excel",
                            &format!("summary-{situation}.xlsx"),
                        )?;
                        generate_summary(
                            &key_dir,
                            job_output_dirs.as_slice(),
                            "--flame",
                            &format!("summary-{situation}"),
                        )?;
                    }

                    Ok(())
                })();

                if let Err(e) = res {
                    info!("done with benchmarking job, but post-evaluation gave an error");
                    Ok(Some(e))
                } else {
                    info!("done with benchmarking job and post-evaluation");
                    Ok(None)
                }
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
        Some(checked_run_parameters),
        "run_job",
    )?;

    if let Some(e) = post_benchmark_error {
        Err(e)
    } else {
        Ok(())
    }
}
