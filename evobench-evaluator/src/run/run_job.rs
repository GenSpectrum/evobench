//! Running a benchmarking job

use std::{
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
    ctx, info,
    io_utils::capture::{CaptureOpts, OutFile},
    key::RunParameters,
    serde::date_and_time::DateTimeWithOffset,
    utillib::info::verbose,
    zstd_file::compress_file,
};

use super::{config::BenchmarkingCommand, working_directory_pool::WorkingDirectoryPool};

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
    checked_run_parameters: RunParameters,
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
    } = &checked_run_parameters;

    let working_directory_id =
        working_directory_pool.get_a_working_directory_for_commit(&commit_id)?;

    working_directory_pool.process_working_directory(
        working_directory_id,
        |working_directory| {
            let timestamp = DateTimeWithOffset::now();

            working_directory.checkout(commit_id.clone())?;

            if dry_run.means(DryRun::DoWorkingDir) {
                println!("checked out working directory: {working_directory_id:?}");
                return Ok(());
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
            let evobench_log = (&bench_tmp_dir).append(format!("evobench-{pid}.log"));
            // File for other output, for optional use by target application
            let bench_output_log = (&bench_tmp_dir).append(format!("bench-output-{pid}.log"));

            let _ = std::fs::remove_file(&evobench_log);

            // for debugging info only:
            let cmd_in_dir = {
                let mut cmd = vec![command.to_string_lossy().into_owned()];
                cmd.append(&mut arguments.clone());
                format!("command {cmd:?} in directory {dir:?}")
            };

            info!("running {cmd_in_dir}, EVOBENCH_LOG={evobench_log:?}...");

            let mut command = Command::new(command);
            command
                .envs(custom_parameters.btree_map())
                .env("EVOBENCH_LOG", &evobench_log)
                .env("BENCH_OUTPUT_LOG", &bench_output_log)
                .args(arguments)
                .current_dir(&dir);

            let command_output_file_path = add_extension(
                working_directory.git_working_dir.working_dir_path_ref(),
                format!("output_of_benchmarking_command_at_{timestamp}"),
            )
            .expect("has filename");
            let command_output_file = OutFile::create(&command_output_file_path)?;

            let mut other_files: Vec<Box<dyn Write + Send + 'static>> = vec![];
            // Evil to use verbose() for this and not a function argument?
            if verbose() {
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
                let result_dir = checked_run_parameters
                    .extend_path(output_base_dir.to_owned())
                    .append(timestamp.as_str());
                std::fs::create_dir_all(&result_dir)
                    .map_err(ctx!("create_dir_all {result_dir:?}"))?;

                info!("running {cmd_in_dir} succeeded; moving files to {result_dir:?}.");

                evobench_evaluator(&vec![
                    "single".into(),
                    (&evobench_log).into(),
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
                    (&evobench_log).into(),
                    "--flame".into(),
                    (&result_dir).append("single").into(),
                ])?;

                let movecompress_file_as = |source_path: &Path,
                                            target_filename: &str|
                 -> Result<()> {
                    let target_filename =
                        add_extension(target_filename, "zstd").expect("got filename");
                    let target = (&result_dir).append(target_filename);
                    compress_file(source_path, &target)?;
                    std::fs::remove_file(source_path).map_err(ctx!("unlink {source_path:?}"))?;
                    Ok(())
                };
                movecompress_file_as(&evobench_log, "evobench.log")?;
                movecompress_file_as(&bench_output_log, "bench_output.log")?;

                Ok(())
            } else {
                info!("running {cmd_in_dir} failed.");
                let last_part = command_output_file.last_part(3000)?;
                if !verbose() {
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
        Some(&checked_run_parameters),
        "checkout",
    )
}
