//! Handle polling the upstream project repository for changes, and
//! also check commit ids for insertions for validity

use std::{collections::BTreeMap, path::Path, str::FromStr, sync::Arc};

use anyhow::Result;
use itertools::Itertools;
use run_git::git::GitWorkingDir;

use crate::{
    git::GitHash,
    serde::{date_and_time::DateTimeWithOffset, git_branch_name::GitBranchName, git_url::GitUrl},
    utillib::arc::CloneArc,
};

use super::{
    config::JobTemplate,
    working_directory_pool::{
        WorkingDirectoryId, WorkingDirectoryPool, WorkingDirectoryPoolBaseDir,
        WorkingDirectoryPoolOpts,
    },
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
        let base_dir =
            WorkingDirectoryPoolBaseDir::new(&opts, &|| unreachable!("already given in opts"))?;
        let pool = WorkingDirectoryPool::open(
            Arc::new(opts),
            base_dir,
            remote_repository_url.clone(),
            true,
        )?;
        Ok(Self { pool })
    }

    /// Updates the remotes, but only if the commit isn't already in
    /// the local clone.
    pub fn commit_is_valid(&mut self, commit: &GitHash) -> Result<bool> {
        self.pool.clear_current_working_directory()?;
        let working_directory_id = self.pool.get_first()?;
        self.pool.process_working_directory(
            working_directory_id,
            &DateTimeWithOffset::now(),
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
        self.pool.clear_current_working_directory()?;
        let working_directory_id = self.pool.get_first()?;
        self.pool.process_working_directory(
            working_directory_id,
            &DateTimeWithOffset::now(),
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
    /// any names that failed to resolve.
    pub fn resolve_branch_names<'b>(
        &mut self,
        working_directory_id: WorkingDirectoryId,
        branch_names: &'b BTreeMap<GitBranchName, Arc<[JobTemplate]>>,
    ) -> Result<(
        Vec<(&'b GitBranchName, GitHash, Arc<[JobTemplate]>)>,
        Vec<String>,
    )> {
        self.pool.process_working_directory(
            working_directory_id,
            &DateTimeWithOffset::now(),
            |working_directory| {
                let mut non_resolving = Vec::new();
                let git_working_dir = &working_directory.git_working_dir;
                let mut ids = Vec::new();
                for (name, job_templates) in branch_names {
                    let ref_string = name.to_ref_string_in_remote("origin");
                    if let Some(id) = git_working_dir.git_rev_parse(&ref_string, true)? {
                        ids.push((name, GitHash::from_str(&id)?, job_templates.clone_arc()))
                    } else {
                        non_resolving.push(ref_string);
                    }
                }
                Ok((ids, non_resolving))
            },
            None,
            &format!("resolving branch names {}", branch_names.keys().join(", ")),
        )
    }
}
