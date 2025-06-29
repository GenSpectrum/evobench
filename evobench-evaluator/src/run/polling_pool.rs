//! Handle polling the upstream project repository for changes, and
//! also check commit ids for insertions for validity

use std::{path::Path, str::FromStr, sync::Arc};

use anyhow::Result;
use run_git::git::{GitCatFileMode, GitWorkingDir};

use crate::{
    git::GitHash,
    serde::{git_branch_name::GitBranchName, git_url::GitUrl},
};

use super::working_directory_pool::{WorkingDirectoryPool, WorkingDirectoryPoolOpts};

fn check_exists(git_working_dir: &GitWorkingDir, commit: &GitHash) -> Result<bool> {
    let commit_str = commit.to_string();
    git_working_dir.git_cat_file(GitCatFileMode::ShowExists, &commit_str)
}

pub struct PollingPool {
    pool: WorkingDirectoryPool,
}

impl PollingPool {
    pub fn open(remote_repository_url: &GitUrl, polling_pool_base: &Path) -> Result<Self> {
        let opts = WorkingDirectoryPoolOpts {
            base_dir: Some(polling_pool_base.to_owned()),
            capacity: 1.try_into().unwrap(),
        };
        let pool = WorkingDirectoryPool::open(
            Arc::new(opts),
            remote_repository_url.clone(),
            true,
            &|| unreachable!("already given in opts"),
        )?;
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

    /// Returns the resolved commit ids for the requested names, and
    /// additionally returns a single string with error messages about
    /// those names that failed to resolve, if any.
    pub fn poll_branch_names(
        &mut self,
        branch_names: &[GitBranchName],
    ) -> Result<(Vec<GitHash>, Option<String>)> {
        if branch_names.is_empty() {
            // XX give an error?
            return Ok((Vec::new(), None));
        }
        let working_directory_id = self.pool.get_first()?;
        let git_url = self.pool.git_url().clone();
        self.pool.process_working_directory(
            working_directory_id,
            |working_directory| {
                let mut errors = Vec::new();
                let git_working_dir = &working_directory.git_working_dir;
                git_working_dir.git(&["remote", "update"], true)?;
                let mut ids = Vec::new();
                for name in branch_names {
                    let ref_string = name.to_ref_string_in_remote("origin");
                    if let Some(id) = git_working_dir.git_rev_parse(&ref_string, true)? {
                        ids.push(GitHash::from_str(&id)?)
                    } else {
                        errors.push(format!(
                            "could not resolve ref string {ref_string:?} in {:?} from {:?}",
                            working_directory.git_working_dir.working_dir_path_ref(),
                            git_url.as_str()
                        ));
                    }
                }
                Ok((
                    ids,
                    if errors.is_empty() {
                        None
                    } else {
                        Some(errors.join(", "))
                    },
                ))
            },
            None,
            &format!("polling branch names {branch_names:?}"),
        )
    }
}
