//! An abstraction for an *existing* directory, and one that should be
//! usable (i.e. is worth trying to use).

use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{anyhow, bail, Result};
use run_git::{
    git::{git_clone, GitResetMode, GitWorkingDir},
    path_util::add_extension,
};
use serde::{Deserialize, Serialize};

use crate::{
    config_file::{load_ron_file, ron_to_file_pretty},
    ctx,
    git::GitHash,
    info,
    serde::{date_and_time::DateTimeWithOffset, git_url::GitUrl},
};

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

    /// Being set aside means, can't be allocated for jobs
    pub fn is_set_aside(self) -> bool {
        match self {
            Status::CheckedOut => false,
            Status::Processing => false,
            Status::Error => true,
            Status::Finished => false,
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
    /// last use time: mtime of the .status file
    pub mtime: SystemTime,
}

impl WorkingDirectory {
    fn status_path_from_working_dir_path(path: &Path) -> Result<PathBuf> {
        add_extension(&path, "status")
            .ok_or_else(|| anyhow!("can't add extension to path {path:?}"))
    }

    fn status_path(&self) -> Result<PathBuf> {
        Self::status_path_from_working_dir_path(self.git_working_dir.working_dir_path_ref())
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        // let quiet = false;

        let status_path = Self::status_path_from_working_dir_path(&path)?;
        let (mtime, status);
        match status_path.metadata() {
            Ok(metadata) => {
                mtime = metadata.modified()?;
                status = load_ron_file(&status_path)?;
            }
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        info!(
                            "note: missing working directory status file {status_path:?}, \
                             creating from defaults"
                        );
                        mtime = SystemTime::now();
                        status = WorkingDirectoryStatus::new();
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
        Ok(Self {
            git_working_dir,
            commit,
            working_directory_status: status,
            mtime,
        })
    }

    /// (Re-)save `$n.status` file (pool is locked (races?, when is
    /// the lock taken?--XXX OH, actually there is no lock!), nobody
    /// can change such a file, thus we do not have to re-check if it
    /// was changed on disk)
    fn save_status(&self) -> Result<()> {
        let status = &self.working_directory_status;
        let path = self.status_path()?;
        ron_to_file_pretty(status, &path, None)?;
        if status.status.is_set_aside() {
            // Mis-use executable bit to easily see error status files
            // in dir listings on the command line.
            std::fs::set_permissions(&path, Permissions::from_mode(0o755))
                .map_err(ctx!("setting executable permission on file {path:?}"))?;
        }
        Ok(())
    }

    /// Give `increment_run_count == true` if you set it to
    /// Status::Processing` (XX do this automatically? But have to
    /// check if it is a change?)
    pub fn change_status(&mut self, to_status: Status, increment_run_count: bool) -> Result<()> {
        self.working_directory_status.status = to_status;
        if increment_run_count {
            self.working_directory_status.num_runs += 1;
        }
        self.save_status()
    }

    pub fn clone_repo(base_dir: &Path, dir_file_name: &str, url: &GitUrl) -> Result<Self> {
        let quiet = false;
        let git_working_dir = git_clone(&base_dir, [], url.as_str(), dir_file_name, quiet)?;
        let commit: GitHash = git_working_dir.get_head_commit_id()?.parse()?;
        let status = WorkingDirectoryStatus::new();
        let mtime = std::fs::metadata(git_working_dir.working_dir_path_ref())?.modified()?;
        info!("clone_repo({base_dir:?}, {dir_file_name:?}, {url}) succeeded");
        let slf = Self {
            git_working_dir,
            commit,
            working_directory_status: status,
            mtime,
        };
        slf.save_status()?;
        Ok(slf)
    }

    /// Checks and is a no-op if already on the commit.
    pub fn checkout(&mut self, commit: GitHash) -> Result<()> {
        let quiet = false;
        let current_commit = self.git_working_dir.get_head_commit_id()?;
        if current_commit == commit.to_string() {
            if self.commit != commit {
                bail!("consistency failure: dir on disk has different commit id from obj")
            }
            Ok(())
        } else {
            let git_working_dir = &self.git_working_dir;
            if !git_working_dir.contains_reference(&commit.to_string())? {
                git_working_dir.git(&["remote", "update"], true)?;
                info!(
                    "checkout({:?}, {commit}): ran git remote update",
                    self.git_working_dir.working_dir_path_ref()
                );
            }

            // First stash, merge --abort, cherry-pick --abort, and all
            // that jazz? No, have such a dir just go set aside with error
            // for manual fixing/removal.
            git_working_dir.git_reset(
                GitResetMode::Hard,
                NO_OPTIONS,
                &commit.to_string(),
                quiet,
            )?;
            info!(
                "checkout({:?}, {commit}): ran git reset --hard",
                self.git_working_dir.working_dir_path_ref()
            );
            self.commit = commit;
            self.working_directory_status.status = Status::CheckedOut;
            self.save_status()?;
            Ok(())
        }
    }
}
