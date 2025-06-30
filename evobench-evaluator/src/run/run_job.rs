//! Running a benchmarking job

use std::{
    io::{stderr, StderrLock, Write},
    process::{Command, Stdio},
};

use anyhow::{bail, Result};
use run_git::path_util::AppendToPath;
use strum_macros::EnumString;

use crate::{ctx, key::RunParameters, utillib::exit_status_ext::ExitStatusExt};

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

pub fn run_job(
    working_directory_pool: &mut WorkingDirectoryPool,
    checked_run_parameters: RunParameters,
    dry_run: DryRun,
    benchmarking_command: &BenchmarkingCommand,
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
            let output = Command::new(command)
                .envs(custom_parameters.btree_map())
                .args(arguments)
                .current_dir(&dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .map_err(ctx!("starting command {command:?} in dir {dir:?}"))?;
            let (status, outputs) = output.status_and_outputs();
            if status.success() {
                Ok(())
            } else {
                {
                    let mut err = stderr().lock();
                    let doit = |err: &mut StderrLock, output: &[u8], ctx: &str| -> Result<()> {
                        writeln!(err, "---- run_job: error in dir {dir:?}: {ctx} -------")?;
                        if output.len() > 3000 {
                            err.write_all("...\n".as_bytes())?;
                            // XX Ignoring UTF-8 boundaries here, evil.
                            err.write_all(&output[output.len() - 3000..])?;
                        } else {
                            err.write_all(output)?;
                        }
                        Ok(())
                    };
                    doit(&mut err, &output.stderr, "stderr")?;
                    doit(&mut err, &output.stdout, "stdout")?;
                    writeln!(&mut err, "---- /run_job: error in dir {dir:?} -------")?;
                }

                let mut cmd = vec![command.to_string_lossy().into_owned()];
                cmd.append(&mut arguments.clone());
                bail!(
                    "benchmarking command {cmd:?} gave error status {status}, outputs {outputs:?}"
                )
            }
        },
        Some(&checked_run_parameters),
        "checkout",
    )
}
