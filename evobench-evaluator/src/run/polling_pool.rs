//! Handle polling the upstream project repository for changes, and
//! also check commit ids for insertions for validity

use std::{path::Path, sync::Arc};

use anyhow::Result;
use run_git::git::{GitCatFileMode, GitWorkingDir};

use crate::{git::GitHash, serde::git_url::GitUrl};

use super::working_directory_pool::{WorkingDirectoryPool, WorkingDirectoryPoolOpts};

fn check_exists(git_working_dir: &GitWorkingDir, commit: &GitHash) -> Result<bool> {
    let commit_str = commit.to_string();
    git_working_dir.git_cat_file(GitCatFileMode::ShowExists, &commit_str)
}

pub struct PollingPool {
    pool: WorkingDirectoryPool,
}

impl PollingPool {
    pub fn open(url: &GitUrl, polling_pool_base: &Path) -> Result<Self> {
        let opts = WorkingDirectoryPoolOpts {
            base_dir: Some(polling_pool_base.to_owned()),
            capacity: 1.try_into().unwrap(),
            url: url.clone(),
        };
        let pool = WorkingDirectoryPool::open(Arc::new(opts), true, &|| {
            unreachable!("already given in opts")
        })?;
        Ok(Self { pool })
    }

    pub fn commit_is_valid(&mut self, commit: &GitHash) -> Result<bool> {
        let working_directory_id = self.pool.get_first()?;
        self.pool.process_working_directory(
            working_directory_id,
            |working_directory| {
                // Check for the commit first, then if it fails, try
                // to update; both for performance, but also to
                // minimize contact with issues with remote server.
                let git_working_dir = &working_directory.git_working_dir;
                Ok(check_exists(git_working_dir, commit)? || {
                    git_working_dir.git(&["remote", "update"], true)?;
                    check_exists(git_working_dir, commit)?
                })
            },
            None,
            &format!("verifying commit {commit}"),
        )
    }
}
