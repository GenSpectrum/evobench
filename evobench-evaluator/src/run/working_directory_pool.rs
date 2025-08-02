//! A pool of `WorkingDirectory`.

//! Error concept: if there are errors, the WorkingDirectory is
//! renamed but stays in the pool directory. (Only directories with
//! names that are parseable as u64 are treated as usable entries.)

use std::{collections::BTreeMap, num::NonZeroU8, path::PathBuf, str::FromStr, sync::Arc, u64};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::{
    ctx,
    git::GitHash,
    info, io_utils,
    key::RunParameters,
    lockable_file::StandaloneExclusiveFileLock,
    path_util::AppendToPath,
    serde::{date_and_time::DateTimeWithOffset, git_url::GitUrl},
};

use super::{run_queues::RunQueuesData, working_directory::WorkingDirectory};

// clap::Args?
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct WorkingDirectoryPoolOpts {
    /// Path to a directory where clones of the project to be
    /// benchmarked should be kept. By default at
    /// `.evobench-run/working_directory_pool/`.
    pub base_dir: Option<PathBuf>,

    /// How many clones of the target project should be maintained;
    /// more is better when multiple commits are benchmarked
    /// alternatively, to avoid needing a rebuild (and input
    /// re-preparation), but costing disk space.
    pub capacity: NonZeroU8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WorkingDirectoryId(u64);

impl WorkingDirectoryId {
    pub fn to_number_string(self) -> String {
        format!("{}", self.0)
    }
    pub fn to_directory_file_name(self) -> String {
        self.to_number_string()
    }
}

pub struct WorkingDirectoryPool {
    opts: Arc<WorkingDirectoryPoolOpts>,
    remote_repository_url: GitUrl,
    // Actual basedir used (opts only has an Option!)
    base_dir: PathBuf,
    next_id: u64,
    entries: BTreeMap<WorkingDirectoryId, WorkingDirectory>,
    /// Only one process may use this pool at the same time
    _lock: StandaloneExclusiveFileLock,
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingError {
    /// An Option since working directory pools are also used for
    /// things that are not benchmark runs
    run_parameters: Option<RunParameters>,
    context: String,
    error: String,
}

impl WorkingDirectoryPool {
    pub fn open(
        opts: Arc<WorkingDirectoryPoolOpts>,
        remote_repository_url: GitUrl,
        create_dir_if_not_exists: bool,
        get_working_directory_pool_base: &dyn Fn() -> Result<PathBuf>,
    ) -> Result<Self> {
        let base_dir = if let Some(path) = opts.base_dir.as_ref() {
            path.to_owned()
        } else {
            get_working_directory_pool_base()?
        };

        if create_dir_if_not_exists {
            io_utils::div::create_dir_if_not_exists(&base_dir, "working pool directory")?;
        }

        let mut next_id: u64 = 0;

        let entries: BTreeMap<WorkingDirectoryId, WorkingDirectory> = std::fs::read_dir(&base_dir)
            .map_err(ctx!("opening working pool directory {base_dir:?}"))?
            .map(
                |entry| -> Result<Option<(WorkingDirectoryId, WorkingDirectory)>> {
                    let entry = entry?;
                    let ft = entry.file_type()?;
                    if !ft.is_dir() {
                        return Ok(None);
                    }
                    let id = if let Some(fname) = entry.file_name().to_str() {
                        if let Some((id_str, _rest)) = fname.split_once('.') {
                            if let Ok(id) = u64::from_str(id_str) {
                                if id >= next_id {
                                    next_id = id + 1;
                                }
                            }
                            return Ok(None);
                        } else {
                            if let Ok(id) = fname.parse() {
                                if id >= next_id {
                                    next_id = id + 1;
                                }
                                WorkingDirectoryId(id)
                            } else {
                                return Ok(None);
                            }
                        }
                    } else {
                        return Ok(None);
                    };
                    let path = entry.path();
                    let wd = WorkingDirectory::open(path)?;
                    Ok(Some((id, wd)))
                },
            )
            .filter_map(|r| r.transpose())
            .collect::<Result<_>>()
            .map_err(ctx!(
                "reading contents of working pool directory {base_dir:?}"
            ))?;

        let lock = StandaloneExclusiveFileLock::try_lock_path(&base_dir, || {
            format!("working directory pool {base_dir:?} is already locked")
        })?;

        let slf = Self {
            opts,
            remote_repository_url,
            base_dir,
            _lock: lock,
            next_id,
            entries,
        };

        info!(
            "opened directory pool {:?} with next_id {next_id}, len {}/{}",
            slf.base_dir,
            slf.len(),
            slf.capacity()
        );

        Ok(slf)
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    /// Guaranteed to be at least 1
    pub fn capacity(&self) -> usize {
        self.opts.capacity.get().into()
    }

    pub fn git_url(&self) -> &GitUrl {
        &self.remote_repository_url
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Always gets a working directory, but doesn't check for any
    /// best fit. If none was cloned yet, that is done now.
    pub fn get_first(&mut self) -> Result<WorkingDirectoryId> {
        if let Some((key, _)) = self.entries.first_key_value() {
            Ok(*key)
        } else {
            self.get_new()
        }
    }

    /// This is *not* public as it is not checking whether there is
    /// capacity left for a new one!
    fn get_new(&mut self) -> Result<WorkingDirectoryId> {
        let id = self.next_id();
        let dir = WorkingDirectory::clone_repo(
            self.base_dir(),
            &id.to_directory_file_name(),
            self.git_url(),
        )?;
        self.entries.insert(id, dir);
        Ok(id)
    }

    pub fn set_processing_error(
        &mut self,
        id: WorkingDirectoryId,
        processing_error: ProcessingError,
        timestamp: &DateTimeWithOffset,
    ) -> Result<()> {
        let old_dir_path = self.base_dir().append(id.to_directory_file_name());
        let new_dir_path = self.base_dir().append(format!(
            "{}.dir_at_{timestamp}",
            id.to_directory_file_name()
        ));
        let error_file_path = self.base_dir().append(format!(
            "{}.error_at_{timestamp}",
            id.to_directory_file_name()
        ));
        let processing_error_string = serde_yml::to_string(&processing_error)?;
        std::fs::rename(&old_dir_path, &new_dir_path)
            .map_err(ctx!("renaming {old_dir_path:?} to {new_dir_path:?}"))?;
        std::fs::write(&error_file_path, &processing_error_string)
            .map_err(ctx!("writing to {error_file_path:?}"))?;
        self.entries.remove(&id);

        info!("set processing error on {id:?}");

        Ok(())
    }

    ///  Runs the given action on the requested working directory and
    ///  with the timestamp of the action start (used in error paths),
    ///  and if there are errors, store them as metadata with the
    ///  directory and remove it from the pool. Returns an error if a
    ///  working directory with the given id doesn't
    ///  exist. `run_parameters` and `context` are only used to be
    ///  stored with the error, if any.
    pub fn process_working_directory<T>(
        &mut self,
        working_directory_id: WorkingDirectoryId,
        action: impl FnOnce(&mut WorkingDirectory, &DateTimeWithOffset) -> Result<T>,
        run_parameters: Option<&RunParameters>,
        context: &str,
    ) -> Result<T> {
        let wd = self
            .entries
            .get_mut(&working_directory_id)
            // Can't just .expect here because the use cases seem too
            // complex (concurrency means that a working directory
            // very well might disappear), thus:
            .ok_or_else(|| anyhow!("working directory id must still exist"))?;

        let timestamp = DateTimeWithOffset::now();

        info!(
            "process_working_directory {working_directory_id:?} \
             ({context}, {run_parameters:?})..."
        );

        match action(wd, &timestamp) {
            Ok(v) => {
                info!(
                    "process_working_directory {working_directory_id:?} \
                     ({context}, {run_parameters:?}) succeeded."
                );

                Ok(v)
            }
            Err(error) => {
                info!(
                    // Do not show error as it might be large; XX
                    // which is a mis-feature!
                    "process_working_directory {working_directory_id:?} \
                     ({context}, {run_parameters:?}) failed."
                );

                let err = format!("{error:#?}");
                self.set_processing_error(
                    working_directory_id,
                    ProcessingError {
                        run_parameters: run_parameters.cloned(),
                        context: context.to_string(),
                        error: err.clone(),
                    },
                    &timestamp,
                )
                .map_err(ctx!("error storing the error {err}"))?;
                Err(error)
            }
        }
    }

    fn next_id(&mut self) -> WorkingDirectoryId {
        let id = self.next_id;
        self.next_id += 1;
        WorkingDirectoryId(id)
    }

    /// Pick a working directory already checked out for the given
    /// commit, and if possible already built or even tested for
    /// it. Returns its id so that the right kind of fresh borrow can
    /// be done.
    fn try_get_best_working_directory_for(
        &self,
        run_parameters: &RunParameters,
        run_queues_data: &RunQueuesData,
    ) -> Option<WorkingDirectoryId> {
        // (todo?: is the working dir used last time for the same job
        // available? Maybe doesn't really matter any more though?)

        let commit: &GitHash = &run_parameters.commit_id;

        // Find one with the same commit
        if let Some((id, _dir)) = self
            .entries
            .iter()
            .filter(|(_, dir)| dir.commit == *commit)
            // Prefer one that proceeded further and is matching
            // closely: todo: store parameters for dir.
            .max_by_key(|(_, dir)| dir.status)
        {
            info!("try_get_best_working_directory_for: found by commit {commit}");
            return Some(*id);
        }

        // Find one that is *not* used by other jobs in the pipeline (i.e. obsolete),
        // and todo: similar parameters
        if let Some((id, _dir)) = self
            .entries
            .iter()
            .filter(|(_, dir)| !run_queues_data.have_entry_with_commit_id(&dir.commit))
            .max_by_key(|(_, dir)| dir.status)
        {
            info!("try_get_best_working_directory_for: found as obsolete");
            return Some(*id);
        }

        None
    }

    /// Return the ~best working directory for the given
    /// run_parameters (e.g. with the requested commit checked out)
    /// and queue pipeline situation (e.g. if forced to change the
    /// checked out commit in a working directory, choose one that
    /// doesn't have a commit checked out that is in the
    /// pipeline). Does *not* check out the commit needed for
    /// run_parameters!
    pub fn get_a_working_directory_for<'s>(
        &'s mut self,
        run_parameters: &RunParameters,
        run_queues_data: &RunQueuesData,
    ) -> Result<WorkingDirectoryId> {
        if let Some(id) = self.try_get_best_working_directory_for(run_parameters, run_queues_data) {
            info!("get_a_working_directory_for -> good old {id:?}");
            Ok(id)
        } else {
            if self.len() < self.capacity() {
                // allocate a new one
                let id = self.get_new()?;
                info!("get_a_working_directory_for -> new {id:?}");
                Ok(id)
            } else {
                // get the least-recently used one
                let id = self
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.mtime)
                    .expect("capacity is guaranteed >= 1")
                    .0
                    .clone();
                info!("get_a_working_directory_for -> lru old {id:?}");
                Ok(id)
            }
        }
    }
}
