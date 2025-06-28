//! Running a benchmarking job

use std::process::Command;

use anyhow::Result;
use strum_macros::EnumString;

use crate::key::RunParameters;

use super::working_directory_pool::WorkingDirectoryPool;

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

            // XXX  build etc run now
            let mut cmd = Command::new("printenv")
                .envs(custom_parameters.btree_map())
                .spawn()?;
            let status = cmd.wait()?;
            dbg!(status);

            Ok(())
        },
        Some(&checked_run_parameters),
        "checkout",
    )
}
