//! A pool of `WorkingDirectory`.

//! Error concept: if there are errors, the WorkingDirectory is
//! renamed but stays in the pool directory. (Only directories with
//! names that are parseable as u64 are treated as usable entries.)

use std::{collections::BTreeMap, num::NonZeroU8, path::PathBuf, str::FromStr, sync::Arc, u64};

use anyhow::Result;
use serde::Serialize;

use crate::{
    ctx,
    git::GitHash,
    info, io_util,
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
    /// benchmarked should be kept. By default at
    /// `.evobench-run/working_directory_pool/`.
    pub base_dir: Option<PathBuf>,

    /// How many clones of the target project should be maintained;
    /// more is better when multiple commits are benchmarked
    /// alternatively, to avoid needing a rebuild (and input
    /// re-preparation), but costing disk space.
    pub capacity: NonZeroU8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkingDirectoryId(u64);

impl WorkingDirectoryId {
    pub fn to_directory_file_name(self) -> String {
        format!("{}", self.0)
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
            io_util::create_dir_if_not_exists(&base_dir, "working pool directory")?;
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
    ) -> Result<()> {
        let now = DateTimeWithOffset::now();
        let old_dir_path = self.base_dir().append(id.to_directory_file_name());
        let new_dir_path = self
            .base_dir()
            .append(format!("{}.error_at_{now}", id.to_directory_file_name()));
        let error_file_path =
            add_extension(&new_dir_path, "processing_error").expect("added file name above");
        let processing_error_string = serde_yml::to_string(&processing_error)?;
        std::fs::rename(&old_dir_path, &new_dir_path)
            .map_err(ctx!("renaming {old_dir_path:?} to {new_dir_path:?}"))?;
        std::fs::write(&error_file_path, &processing_error_string)
            .map_err(ctx!("writing to {error_file_path:?}"))?;
        self.entries.remove(&id);

        info!("set processing error on {id:?}");

        Ok(())
    }

    ///  Runs the given action on the requested working directory, and
    ///  if there are errors, store them as metadata with the
    ///  directory and remove it from the pool. Panics if a working
    ///  directory with the given id doesn't exist. `run_parameters`
    ///  and `context` are only used to be stored with the error, if
    ///  any.
    pub fn process_working_directory<T>(
        &mut self,
        working_directory_id: WorkingDirectoryId,
        action: impl FnOnce(&mut WorkingDirectory) -> Result<T>,
        run_parameters: Option<&RunParameters>,
        context: &str,
    ) -> Result<T> {
        let wd = self
            .entries
            .get_mut(&working_directory_id)
            .expect("working directory id must still exist");

        info!(
            "process_working_directory {working_directory_id:?} \
             ({context}, {run_parameters:?})..."
        );

        match action(wd) {
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
    pub fn try_get_best_working_directory_for_commit(
        &self,
        commit: &GitHash,
    ) -> Option<WorkingDirectoryId> {
        let mut dirs: Vec<(&WorkingDirectoryId, &WorkingDirectory)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.commit == *commit)
            .collect();
        dirs.sort_by_key(|(_, entry)| entry.status);
        let res = dirs.last().map(|(id, _)| **id);

        info!(
            "try_get_best_working_directory_for_commit({:?}, {commit}) returning {res:?}",
            self.base_dir
        );

        res
    }

    /// Return a working directory with the requested commit checked
    /// out (but no build or other action carried out). Returns
    /// existing entry for the commit if available, otherwise makes a
    /// new clone if the configured capacity hasn't been reached or
    /// returns the least-recently used clone (XX or closest to the
    /// commit?). Does *not* check out the requested commit!
    pub fn get_a_working_directory_for_commit<'s>(
        &'s mut self,
        commit: &'s GitHash,
    ) -> Result<WorkingDirectoryId> {
        if let Some(id) = self.try_get_best_working_directory_for_commit(commit) {
            info!("get_a_working_directory_for_commit({commit}) -> old {id:?}");
            Ok(id)
        } else {
            if self.len() < self.capacity() {
                // allocate a new one
                let id = self.get_new()?;
                info!("get_a_working_directory_for_commit({commit}) -> new {id:?}");
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
                info!("get_a_working_directory_for_commit({commit}) -> lru {id:?}");
                Ok(id)
            }
        }
    }
}
