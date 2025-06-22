//! An abstraction for an *existing* directory, and one that should be
//! usable (i.e. is worth trying to use)

use std::{path::PathBuf, time::SystemTime};

use anyhow::Result;

use crate::{git::GitHash, serde::git_url::GitUrl};

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
    pub path: PathBuf,
    pub commit: GitHash,
    pub status: Status,
    /// last use time: mtime of the folder, which is touched on every
    /// use, too
    pub mtime: SystemTime,
}

impl WorkingDirectory {
    pub fn open(path: PathBuf) -> Result<Self> {
        todo!()
    }

    pub fn clone_repo(path: PathBuf, url: &GitUrl) -> Result<Self> {
        // git_clone_to(&path, url)?;
        // let commit = git_rev_parse(&path, "HEAD")?.parse()?;
        let status = Status::CheckedOut;
        // Ok(Self {
        //     path,
        //     commit,
        //     status,
        // })
        todo!()
    }

    pub fn checkout(&mut self, commit: GitHash) -> Result<()> {
        // First stash, merge --abort, cherry-pick --abort, and all
        // that jazz? No, have such a dir just go set aside with error
        // for manual fixing/removal.
        // git(&self.path, ["checkout", "-b", branch_name]);
        todo!()
    }
}
