//! If there are errors, the WorkingDirectory is renamed but stays in
//! the pool directory. I.e. only directories with names that are
//! parseable as u64 are treated as usable entries.

use std::{collections::BTreeMap, num::NonZeroU8, path::PathBuf, u64};

use anyhow::Result;
use serde::Serialize;

use crate::{
    ctx,
    git::GitHash,
    io_util,
    key::RunParameters,
    lockable_file::StandaloneExclusiveFileLock,
    path_util::{add_extension, AppendToPath},
    serde::{date_and_time::DateTimeWithOffset, git_url::GitUrl},
};

use super::working_directory::WorkingDirectory;

// clap::Args?
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct WorkingDirectoryPoolOpts {
    /// Path to a directory where clones of the project to be
    /// benchmarked should be kept
    pub base_dir: PathBuf,

    /// How many clones of the target project should be maintained;
    /// more is better when multiple commits are benchmarked
    /// alternatively, to avoid needing a rebuild (and input
    /// re-preparation), but costing disk space.
    pub capacity: NonZeroU8,

    /// The Git URL from where to clone the target project
    pub url: GitUrl,
}

pub struct WorkingDirectoryPool {
    opts: WorkingDirectoryPoolOpts,
    next_id: u64,
    entries: BTreeMap<u64, WorkingDirectory>,
    /// Only one process may use this pool at the same time
    _lock: StandaloneExclusiveFileLock,
}

#[derive(Debug, Serialize)]
pub struct ProcessingError {
    run_parameters: RunParameters,
    context: String,
    error: String,
}

impl WorkingDirectoryPool {
    pub fn open(opts: WorkingDirectoryPoolOpts, create_dir_if_not_exists: bool) -> Result<Self> {
        let path = &opts.base_dir;

        if create_dir_if_not_exists {
            io_util::create_dir_if_not_exists(path, "working pool directory")?;
        }

        let entries = std::fs::read_dir(path)
            .map_err(ctx!("opening working pool directory {path:?}"))?
            .map(|entry| -> Result<Option<(u64, WorkingDirectory)>> {
                let entry = entry?;
                let ft = entry.file_type()?;
                if !ft.is_dir() {
                    return Ok(None);
                }
                let id = if let Some(fname) = entry.file_name().to_str() {
                    if let Ok(id) = fname.parse() {
                        id
                    } else {
                        return Ok(None);
                    }
                } else {
                    return Ok(None);
                };
                let path = entry.path();
                let wd = WorkingDirectory::open(path)?;
                Ok(Some((id, wd)))
            })
            .filter_map(|r| r.transpose())
            .collect::<Result<_>>()
            .map_err(ctx!("reading contents of working pool directory {path:?}"))?;

        let lock = StandaloneExclusiveFileLock::try_lock_path(path, || {
            format!("working directory pool {path:?} is already locked")
        })?;

        Ok(Self {
            opts,
            _lock: lock,
            next_id: 0,
            entries,
        })
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.opts.base_dir
    }

    /// Guaranteed to be at least 1
    pub fn capacity(&self) -> usize {
        self.opts.capacity.get().into()
    }

    pub fn git_url(&self) -> &GitUrl {
        &self.opts.url
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn set_processing_error(
        &mut self,
        id: u64,
        processing_error: ProcessingError,
    ) -> Result<()> {
        let now = DateTimeWithOffset::now();
        let old_dir_path = self.base_dir().append(format!("{id}"));
        let new_dir_path = self.base_dir().append(format!("{id}.error_at_{now}"));
        let error_file_path =
            add_extension(&new_dir_path, "processing_error").expect("added file name above");
        let processing_error_string = serde_yml::to_string(&processing_error)?;
        std::fs::rename(&old_dir_path, &new_dir_path)
            .map_err(ctx!("renaming {old_dir_path:?} to {new_dir_path:?}"))?;
        std::fs::write(&error_file_path, &processing_error_string)
            .map_err(ctx!("writing to {error_file_path:?}"))
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // /// All working directories (ids) checked out for the given commit
    // pub fn get_working_directories_by_commit<'s>(
    //     &'s self,
    //     commit: &'s GitHash,
    // ) -> impl Iterator<Item = u64> + use<'s> {
    //     self.entries
    //         .iter()
    //         .filter_map(|(id, entry)| if entry.commit == *commit { Some(*id) } else { None })
    // }

    /// Pick a working directory already checked out for the given
    /// commit, and if possible already built or even tested for
    /// it. Returns its id so that the right kind of fresh borrow can
    /// be done.
    pub fn try_get_best_working_directory_for_commit(&self, commit: &GitHash) -> Option<u64> {
        let mut dirs: Vec<(&u64, &WorkingDirectory)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.commit == *commit)
            .collect();
        dirs.sort_by_key(|(_, entry)| entry.status);
        dirs.last().map(|(id, _)| **id)
    }

    /// Return a working directory with the requested commit checked
    /// out (but no build or other action carried out). Returns
    /// existing entry for the commit if available, otherwise makes a
    /// new clone if the configured capacity hasn't been reached or
    /// returns the least-recently used clone (XX or closest to the
    /// commit?). Does *not* check out the requested commit!
    pub fn get_working_directory_for_commit<'s>(
        &'s mut self,
        commit: &'s GitHash,
    ) -> Result<&'s mut WorkingDirectory> {
        if let Some(id) = self.try_get_best_working_directory_for_commit(commit) {
            Ok(self.entries.get_mut(&id).expect("just got the id"))
        } else {
            if self.len() < self.capacity() {
                // allocate a new one
                let id = self.next_id();
                let path = self.base_dir().append(format!("{id}"));
                let dir = WorkingDirectory::clone_repo(path, self.git_url())?;
                self.entries.insert(id, dir);
                Ok(self.entries.get_mut(&id).unwrap())
            } else {
                // get the least-recently used one
                Ok(self
                    .entries
                    .values_mut()
                    .min_by_key(|entry| entry.mtime)
                    .expect("capacity is guaranteed >= 1"))
            }
        }
    }
}
