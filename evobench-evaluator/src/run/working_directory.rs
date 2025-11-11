//! An abstraction for an *existing* directory, and one that should be
//! usable (i.e. is worth trying to use).

use std::{
    fmt::Display,
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{anyhow, bail, Result};
use run_git::{
    git::{git_clone, GitResetMode, GitWorkingDir},
    path_util::add_extension,
};
use serde::{Deserialize, Serialize};

use crate::{
    config_file::{load_ron_file, ron_to_file_pretty},
    ctx, debug,
    git::GitHash,
    info,
    serde::{date_and_time::DateTimeWithOffset, git_url::GitUrl},
};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename = "WorkingDirectoryAutoClean")]
pub struct WorkingDirectoryAutoCleanOpts {
    /// The minimum age a working directory should reach before
    /// possibly being deleted, in days (recommended: 3)
    pub min_age_days: u16,

    /// The minimum number of jobs that should be run in a working
    /// directory before that is possibly being deleted (recommended:
    /// 80).
    pub min_num_runs: usize,

    /// If true, directories are not deleted when any job for the same
    /// commit id is in the queue.  (Directories are deleted when they
    /// reach both the `min_age_days` and `min_num_runs` numbers, and
    /// this is false, or the current job just ended and no others for
    /// the commit id exist.)
    pub wait_until_commit_done: bool,
}

const NO_OPTIONS: &[&str] = &[];

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Only checked out for that commit
    CheckedOut,
    /// Currently running in process_working_directory
    Processing,
    /// Project benchmarking gave error for that commit (means working dir is set aside)
    Error,
    /// Project benchmarking ran through for that commit
    Finished,
}

impl Status {
    /// How well the dir is usable for a given commit id
    fn score(self) -> u32 {
        match self {
            Status::CheckedOut => 1,
            Status::Processing => 2,
            Status::Error => 3,
            Status::Finished => 4,
        }
    }

    /// True means, can't be allocated for jobs
    pub fn is_error(self) -> bool {
        match self {
            Status::CheckedOut => false,
            Status::Processing => false,
            Status::Error => true,
            Status::Finished => false,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::CheckedOut => "checked-out",
            Status::Processing => "processing",
            Status::Error => "error",
            Status::Finished => "finished",
        }
    }
}

impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Status {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.score().cmp(&other.score())
    }
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stored in `$n.status` files for each working directory
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkingDirectoryStatus {
    pub creation_timestamp: DateTimeWithOffset,
    pub num_runs: usize,
    pub status: Status,
}

impl WorkingDirectoryStatus {
    fn new() -> Self {
        Self {
            creation_timestamp: DateTimeWithOffset::now(),
            num_runs: 0,
            status: Status::CheckedOut,
        }
    }
}

#[derive(Debug)]
pub struct WorkingDirectory {
    pub git_working_dir: GitWorkingDir,
    pub commit: GitHash,
    pub working_directory_status: WorkingDirectoryStatus,
    working_directory_status_needs_saving: bool,
    /// last use time: mtime of the .status file
    pub last_use: SystemTime,
}

impl WorkingDirectory {
    pub fn status_path_from_working_dir_path(path: &Path) -> Result<PathBuf> {
        add_extension(&path, "status")
            .ok_or_else(|| anyhow!("can't add extension to path {path:?}"))
    }

    fn status_path(&self) -> Result<PathBuf> {
        Self::status_path_from_working_dir_path(self.git_working_dir.working_dir_path_ref())
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        // let quiet = false;

        let working_directory_status_needs_saving;

        let status_path = Self::status_path_from_working_dir_path(&path)?;
        let (mtime, working_directory_status);
        match status_path.metadata() {
            Ok(metadata) => {
                mtime = metadata.modified()?;
                working_directory_status = load_ron_file(&status_path)?;
                working_directory_status_needs_saving = false;
            }
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        info!(
                            "note: missing working directory status file {status_path:?}, \
                             creating from defaults"
                        );
                        mtime = SystemTime::now();
                        working_directory_status = WorkingDirectoryStatus::new();
                        working_directory_status_needs_saving = true;
                    }
                    _ => {
                        return Err(e).map_err(ctx!(
                            "checking working directory status file path {status_path:?}"
                        ))
                    }
                };
            }
        }

        let git_working_dir = GitWorkingDir::from(path);
        {
            let path = git_working_dir.working_dir_path_ref();
            std::fs::metadata(path)
                .map_err(ctx!("WorkingDirectory::open({path:?})"))?
                .modified()?
        };
        let commit: GitHash = git_working_dir.get_head_commit_id()?.parse()?;
        let status = working_directory_status.status;
        let mut slf = Self {
            git_working_dir,
            commit,
            working_directory_status,
            working_directory_status_needs_saving,
            last_use: mtime,
        };
        // XX chaos: Do not change the status if it already
        // exists. Does this even work?
        slf.set_and_save_status(status)?;
        Ok(slf)
    }

    /// Set status to `status`. Also increments the run count if the
    /// status changed to Status::Processing, and (re-)saves
    /// `$n.status` file if needed.
    // (pool is locked (races?, when is
    // the lock taken?--XXX OH, actually there is no lock!), nobody
    // can change such a file, thus we do not have to re-check if it
    // was changed on disk)
    pub fn set_and_save_status(&mut self, status: Status) -> Result<()> {
        debug!("{:?} set_and_save_status({status:?})", self.git_working_dir);
        let old_status = self.working_directory_status.status;
        self.working_directory_status.status = status;
        let needs_saving;
        if old_status != status {
            needs_saving = true;
            if status == Status::Processing {
                self.working_directory_status.num_runs += 1;
            }
        } else {
            needs_saving = self.working_directory_status_needs_saving;
        }
        if needs_saving {
            let working_directory_status = &self.working_directory_status;
            let path = self.status_path()?;
            ron_to_file_pretty(working_directory_status, &path, false, None)?;
            if working_directory_status.status.is_error() {
                // Mis-use executable bit to easily see error status files
                // in dir listings on the command line.
                std::fs::set_permissions(&path, Permissions::from_mode(0o755))
                    .map_err(ctx!("setting executable permission on file {path:?}"))?;
            }
            debug!(
                "{:?} set_and_save_status({status:?}): file saved",
                self.git_working_dir
            );
        }
        self.working_directory_status_needs_saving = false;
        Ok(())
    }

    pub fn clone_repo(base_dir: &Path, dir_file_name: &str, url: &GitUrl) -> Result<Self> {
        let quiet = false;
        let git_working_dir = git_clone(&base_dir, [], url.as_str(), dir_file_name, quiet)?;
        let commit: GitHash = git_working_dir.get_head_commit_id()?.parse()?;
        let status = WorkingDirectoryStatus::new();
        let mtime = status.creation_timestamp.to_systemtime();
        info!("clone_repo({base_dir:?}, {dir_file_name:?}, {url}) succeeded");
        let mut slf = Self {
            git_working_dir,
            commit,
            working_directory_status: status,
            working_directory_status_needs_saving: true,
            last_use: mtime,
        };
        slf.set_and_save_status(Status::CheckedOut)?;
        Ok(slf)
    }

    /// Checks and is a no-op if already on the commit.
    pub fn checkout(&mut self, commit: GitHash) -> Result<()> {
        let commit_str = commit.to_string();
        let quiet = false;
        let current_commit = self.git_working_dir.get_head_commit_id()?;
        if current_commit == commit_str {
            if self.commit != commit {
                bail!("consistency failure: dir on disk has different commit id from obj")
            }
            Ok(())
        } else {
            let git_working_dir = &self.git_working_dir;
            if !git_working_dir.contains_reference(&commit_str)? {
                // XX really rely on "origin"? Seems we don't have a
                // way to know or even query the default remote?  But
                // it should be safe as long as we freshly clone those
                // repositories. Fetching --tags in case
                // `dataset_dir_for_commit` is used. Note: this does
                // not update branches, right? But branch names should
                // never be used for anything, OK? XX document?
                git_working_dir.git(&["fetch", "origin", "--tags", &commit_str], true)?;
                info!(
                    "checkout({:?}, {commit}): ran git fetch origin --tags {commit_str}",
                    self.git_working_dir.working_dir_path_ref()
                );
            }

            // First stash, merge --abort, cherry-pick --abort, and all
            // that jazz? No, have such a dir just go set aside with error
            // for manual fixing/removal.
            git_working_dir.git_reset(GitResetMode::Hard, NO_OPTIONS, &commit_str, quiet)?;
            info!(
                "checkout({:?}, {commit}): ran git reset --hard",
                self.git_working_dir.working_dir_path_ref()
            );
            self.commit = commit;
            self.set_and_save_status(Status::CheckedOut)?;
            Ok(())
        }
    }

    pub fn needs_cleanup(
        &self,
        opts: Option<&WorkingDirectoryAutoCleanOpts>,
        have_other_jobs_for_same_commit: Option<&dyn Fn() -> bool>,
    ) -> Result<bool> {
        if let Some(WorkingDirectoryAutoCleanOpts {
            min_age_days,
            min_num_runs,
            wait_until_commit_done,
        }) = opts
        {
            let is_old_enough = {
                let min_age_days: u64 = (*min_age_days).into();
                let min_age = Duration::from_secs(24 * 3600 * min_age_days);
                let now = SystemTime::now();
                let creation_time: SystemTime = self
                    .working_directory_status
                    .creation_timestamp
                    .to_systemtime();
                let age = now.duration_since(creation_time).map_err(ctx!(
                    "calculating age for working directory {:?}",
                    self.git_working_dir.working_dir_path_ref()
                ))?;
                age >= min_age
            };
            let is_used_enough = { self.working_directory_status.num_runs >= *min_num_runs };
            Ok(is_old_enough
                && is_used_enough
                && ((!*wait_until_commit_done) || {
                    if let Some(have_other_jobs_for_same_commit) = have_other_jobs_for_same_commit {
                        have_other_jobs_for_same_commit()
                    } else {
                        // Could actually short-cut the calls from
                        // polling_pool.rs to false here. But making those
                        // configurable may still be good.
                        true
                    }
                }))
        } else {
            info!(
                "never cleaning up working directories since there is no \
             `auto_clean` configuration"
            );
            Ok(false)
        }
    }
}
