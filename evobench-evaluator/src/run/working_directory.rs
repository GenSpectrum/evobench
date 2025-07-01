//! An abstraction for an *existing* directory, and one that should be
//! usable (i.e. is worth trying to use).

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{bail, Result};
use run_git::git::{git_clone, GitResetMode, GitWorkingDir};

use crate::{ctx, git::GitHash, info, serde::git_url::GitUrl};

const NO_OPTIONS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    CheckedOut,
    Built,
    Benchmarked,
}

impl Status {
    pub fn value_scrore(self) -> u32 {
        match self {
            Status::CheckedOut => 1,
            Status::Built => 2,
            Status::Benchmarked => 3,
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
        self.value_scrore().cmp(&other.value_scrore())
    }
}

#[derive(Debug)]
pub struct WorkingDirectory {
    pub git_working_dir: GitWorkingDir,
    pub commit: GitHash,
    pub status: Status,
    /// last use time: mtime of the folder, which is touched on every
    /// use, too
    pub mtime: SystemTime,
}

impl WorkingDirectory {
    pub fn open(path: PathBuf) -> Result<Self> {
        // let quiet = false;
        let git_working_dir = GitWorkingDir::from(path);
        let mtime = {
            let path = git_working_dir.working_dir_path_ref();
            std::fs::metadata(path)
                .map_err(ctx!("WorkingDirectory::open({path:?})"))?
                .modified()?
        };
        let commit: GitHash = git_working_dir.get_head_commit_id()?.parse()?;
        let status = Status::CheckedOut;
        Ok(Self {
            git_working_dir,
            commit,
            status,
            mtime,
        })
    }

    pub fn clone_repo(base_dir: &Path, dir_file_name: &str, url: &GitUrl) -> Result<Self> {
        let quiet = false;
        let git_working_dir = git_clone(&base_dir, [], url.as_str(), dir_file_name, quiet)?;
        let commit: GitHash = git_working_dir.get_head_commit_id()?.parse()?;
        let status = Status::CheckedOut;
        let mtime = std::fs::metadata(git_working_dir.working_dir_path_ref())?.modified()?;
        info!("clone_repo({base_dir:?}, {dir_file_name:?}, {url}) succeeded");
        Ok(Self {
            git_working_dir,
            commit,
            status,
            mtime,
        })
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
            self.status = Status::CheckedOut;
            Ok(())
        }
    }
}
