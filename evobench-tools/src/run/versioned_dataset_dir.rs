use std::{
    collections::HashMap,
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, MutexGuard},
};

use anyhow::{Result, anyhow, bail};
use cj_path_util::path_util::AppendToPath;
use run_git::git::GitWorkingDir;

use crate::{
    ctx, debug,
    git::{GitGraph, GitGraphData, GitHash},
    serde::proper_filename::ProperFilename,
    warn,
};

/// Index of all versioned entries
#[derive(Debug)]
pub struct VersionedDatasetReferencesIndex {
    commit_to_dirname: HashMap<GitHash, ProperFilename>,
}

impl VersionedDatasetReferencesIndex {
    pub fn read(versioned_datasets_dir: &Path, git_working_dir: &GitWorkingDir) -> Result<Self> {
        let mut commit_to_dirname = HashMap::new();
        for entry in std::fs::read_dir(&versioned_datasets_dir)
            .map_err(ctx!("can't open directory {versioned_datasets_dir:?}"))?
        {
            let entry = entry?;
            // Ignore non-dir entries? Could allow for README
            // files or so.
            if entry.path().is_file() {
                continue;
            }
            if !entry.path().is_dir() {
                bail!(
                    "non-dir non-file entry (broken symlink?) at {:?}",
                    entry.path()
                )
            }
            if let Some(file_name) = entry.file_name().to_str() {
                if let Some(commit) = git_working_dir.git_rev_parse(&file_name, true)? {
                    // XX can git_rev_parse not return GitHash? Wrap
                    // it when called with true?
                    let commit = GitHash::from_str(&commit)?;
                    let folder_name: ProperFilename = file_name.parse().map_err(|e| {
                        anyhow!(
                            "versioned dataset dir {versioned_datasets_dir:?} \
                             entry {file_name:?}: {e:#}"
                        )
                    })?;
                    commit_to_dirname.insert(commit, folder_name);
                } else {
                    warn!(
                        "file name of this path can't be found as Git revision: {:?}",
                        entry.path()
                    )
                }
            } else {
                warn!(
                    "file name of this path can't be decoded as utf-8: {:?}",
                    entry.path()
                )
            }
        }
        Ok(Self { commit_to_dirname })
    }
}

impl Deref for VersionedDatasetReferencesIndex {
    type Target = HashMap<GitHash, ProperFilename>;

    fn deref(&self) -> &Self::Target {
        &self.commit_to_dirname
    }
}

pub struct VersionedDatasetDir {
    git_graph: Arc<GitGraph>,
}

impl VersionedDatasetDir {
    pub fn new() -> Self {
        Self {
            git_graph: GitGraph::new(),
        }
    }

    pub fn updated_git_graph<'s>(
        &'s self,
        git_working_dir: &'s GitWorkingDir,
        commit_id: &'s GitHash,
    ) -> Result<VersionedDatasetDirLock<'s>> {
        // Update graphdata with the (no need to update
        // tag mappings, they are read directly from Git
        // in `dataset_dir_for_commit`)
        let mut git_graph_data = self.git_graph.lock();
        git_graph_data.add_history_from_dir_ref(
            // XX should pass &git_working_dir instead
            &*git_working_dir.working_dir_path,
            // XX should pass &GitHash instead
            &commit_id.to_string(),
        )?;
        Ok(VersionedDatasetDirLock {
            git_working_dir,
            commit_id,
            git_graph_data,
        })
    }
}

pub struct VersionedDatasetDirLock<'s> {
    git_working_dir: &'s GitWorkingDir,
    commit_id: &'s GitHash,
    git_graph_data: MutexGuard<'s, GitGraphData>,
}

impl<'s> VersionedDatasetDirLock<'s> {
    /// Must be up to date and include all the possibly used
    /// references and the commit! Those conditions are ensured by
    /// `working_directory.checkout()`.
    pub fn dataset_dir_for_commit(
        &self,
        versioned_datasets_base_dir: &Path,
        dataset_name: &str,
    ) -> Result<PathBuf> {
        let versioned_datasets_dir = versioned_datasets_base_dir.append(dataset_name);

        let commit_to_dirname =
            VersionedDatasetReferencesIndex::read(&versioned_datasets_dir, self.git_working_dir)?;
        let commit_id = self.commit_id;
        debug!(
            "of the revisions {commit_to_dirname:?}, \
             find the latest ancestor for commit {commit_id:?}"
        );
        let commit_id_id = self
            .git_graph_data
            .get_by_hash(commit_id)
            .expect("always contained, as per documented usage contract");
        let ancestor_or_self_id = self
            .git_graph_data
            .closest_matching_ancestor_of(commit_id_id, |id| {
                let commit = self.git_graph_data.get(id).expect("internal consistency");
                commit_to_dirname.contains_key(&commit.commit.commit_hash)
            })?
            .ok_or_else(|| {
                anyhow!(
                    "can't find a dataset for commit {commit_id} in dir {versioned_datasets_dir:?} \
                     -- datasets should be in sub-directories of this dir, named after Git \
                     references (like tags or commits)"
                )
            })?;
        let ancestor_or_self = &self
            .git_graph_data
            .get(ancestor_or_self_id)
            .expect("internal consistency")
            .commit
            .commit_hash;
        let chosen_dirname = commit_to_dirname
            .get(ancestor_or_self)
            .expect("outer internal consistency");
        Ok(versioned_datasets_dir.append(chosen_dirname.as_str()))
    }
}
