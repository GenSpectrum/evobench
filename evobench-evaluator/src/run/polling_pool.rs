//! Handle polling the upstream project repository for changes, and
//! also check commit ids for insertions for validity

use std::{path::Path, str::FromStr, sync::Arc};

use anyhow::Result;
use run_git::git::GitWorkingDir;

use crate::{
    git::GitHash,
    serde::{git_branch_name::GitBranchName, git_url::GitUrl},
};

use super::working_directory_pool::{
    WorkingDirectoryId, WorkingDirectoryPool, WorkingDirectoryPoolOpts,
};

fn check_exists(git_working_dir: &GitWorkingDir, commit: &GitHash) -> Result<bool> {
    let commit_str = commit.to_string();
    git_working_dir.contains_reference(&commit_str)
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

    /// Updates the remotes, but only if the commit isn't already in
    /// the local clone.
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

    /// Get working dir, git remote update it, and return its id for
    /// subsequent work on it
    pub fn updated_working_dir(&mut self) -> Result<WorkingDirectoryId> {
        let working_directory_id = self.pool.get_first()?;
        self.pool.process_working_directory(
            working_directory_id,
            |working_directory| {
                let git_working_dir = &working_directory.git_working_dir;
                git_working_dir.git(&["remote", "update"], true)?;
                Ok(working_directory_id)
            },
            None,
            "updated_working_dir()",
        )
    }

    /// Returns the resolved commit ids for the requested names, and
    /// additionally returns a single string with error messages about
    /// those names that failed to resolve, if any.
    pub fn resolve_branch_names(
        &mut self,
        working_directory_id: WorkingDirectoryId,
        branch_names: &[GitBranchName],
    ) -> Result<(Vec<GitHash>, Option<String>)> {
        let git_url = self.pool.git_url().clone();
        self.pool.process_working_directory(
            working_directory_id,
            |working_directory| {
                let mut errors = Vec::new();
                let git_working_dir = &working_directory.git_working_dir;
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
            &format!("resolving branch names {branch_names:?}"),
        )
    }
}
