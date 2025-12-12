//! A pool of `WorkingDirectory`.

//! Error concept: if there are errors, the WorkingDirectory is
//! renamed but stays in the pool directory. (Only directories with
//! names that are parseable as u64 are treated as usable entries.)

use std::{
    collections::BTreeMap,
    fmt::Display,
    fs::File,
    num::NonZeroU8,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    u64,
};

use anyhow::{anyhow, bail, Result};
use cj_path_util::path_util::AppendToPath;
use serde::{Deserialize, Serialize};

use crate::{
    config_file::load_ron_file,
    ctx, debug, def_linear,
    git::GitHash,
    info, io_utils,
    key::{BenchmarkingJobParameters, RunParameters},
    owning_lockable_file::{OwningExclusiveFileLock, OwningLockableFile},
    run::working_directory::{
        WorkingDirectoryAutoCleanOpts, WorkingDirectoryWithPoolLock,
        WorkingDirectoryWithPoolLockMut, WorkingDirectoryWithPoolMut,
    },
    serde::{date_and_time::DateTimeWithOffset, git_url::GitUrl},
    utillib::arc::CloneArc,
};

use super::{
    run_queues::RunQueuesData,
    working_directory::{Status, WorkingDirectory, WorkingDirectoryStatus},
};

// clap::Args?
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename = "WorkingDirectoryPool")]
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

    /// To enable working directory auto-cleaning, give the
    /// cleaning options. Currently "cleaning" just means full
    /// deletion by the runner with no involvement of the target
    /// project.
    pub auto_clean: Option<WorkingDirectoryAutoCleanOpts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WorkingDirectoryId(u64);

def_linear!(Linear in WorkingDirectoryCleanupToken);

/// This is a linear type (i.e. it cannot be dropped) and has to be
/// passed to `working_directory_cleanup`, which will potentially
/// clean up or delete the working directory that this represents. If
/// you want to prevent that, call `prohibit_cleanup()` on it before
/// passing it, or call its `force_drop()` method (easier to do in
/// error handlers).
#[must_use]
pub struct WorkingDirectoryCleanupToken {
    linear_token: Linear,
    working_directory_id: WorkingDirectoryId,
    needs_cleanup: bool,
}
// For impl WorkingDirectoryCleanupToken: `force_drop` and
// `prohibiting_cleanup` methods, see git history.

impl WorkingDirectoryId {
    fn to_number_string(self) -> String {
        format!("{}", self.0)
    }
    pub fn to_directory_file_name(self) -> String {
        self.to_number_string()
    }
    pub fn from_prefixless_str(s: &str) -> Result<Self> {
        let id = s.parse()?;
        Ok(Self(id))
    }
}

impl Display for WorkingDirectoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "D{}", self.0)
    }
}

impl FromStr for WorkingDirectoryId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let number = s
            .strip_prefix("D")
            .or_else(|| s.strip_prefix("d"))
            .ok_or_else(|| anyhow!("missing 'D' at beginning of working directory ID"))?;
        let id = number.parse()?;
        Ok(Self(id))
    }
}

#[derive(Debug)]
pub struct WorkingDirectoryPoolBaseDir {
    path: Arc<Path>,
    dir_file: OwningLockableFile<File>,
}

impl WorkingDirectoryPoolBaseDir {
    pub fn new(
        opts: &WorkingDirectoryPoolOpts,
        get_working_directory_pool_base: &dyn Fn() -> Result<PathBuf>,
    ) -> Result<Self> {
        let path: Arc<Path> = if let Some(path) = opts.base_dir.as_ref() {
            path.to_owned()
        } else {
            get_working_directory_pool_base()?
        }
        .into();
        let dir_file = OwningLockableFile::open(path.clone_arc())
            .map_err(ctx!("opening working directory base dir {path:?}"))?;
        Ok(Self { path, dir_file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path to the symlink to the currently used working
    /// directory
    fn current_working_directory_symlink_path(&self) -> PathBuf {
        self.path().append("current")
    }

    /// Lock the base dir of the pool, blocking (this is *not* the
    /// global job-running lock any more!)
    // XX *not* &mut, bc dont need, keep it private, ok?, yet won't
    // help the problem anyway. Anyway, keep private, since
    // OwningExclusiveFileLock does not have a lifetime!
    fn get_lock<'s>(&'s self, locker: &str) -> Result<OwningExclusiveFileLock<File>> {
        let path = self.path();
        debug!(
            "getting working directory pool lock on {:?} for {locker}",
            self.path(),
        );
        self.dir_file
            .lock_exclusive()
            .map_err(ctx!("locking working directory pool base dir {path:?}"))
    }

    /// Lock the base dir of the pool, blocking (this is *not* the
    /// global job-running lock any more!)
    pub fn lock<'s>(&'s self, locker: &str) -> Result<WorkingDirectoryPoolBaseDirLock<'s>> {
        let lock = self.get_lock(locker)?;
        Ok(WorkingDirectoryPoolBaseDirLock {
            base_dir: self,
            _lock: Some(lock),
        })
    }
}

/// A public lock on a WorkingDirectoryPoolBaseDir. Takes exclusive
/// access to the WorkingDirectoryPoolBaseDir.
// do *not* allow to clone, or even move or share between threads, OK?
pub struct WorkingDirectoryPoolBaseDirLock<'t> {
    base_dir: &'t WorkingDirectoryPoolBaseDir,
    _lock: Option<OwningExclusiveFileLock<File>>,
}

impl<'t> WorkingDirectoryPoolBaseDirLock<'t> {
    /// Read the working directory from symlink, if present
    pub fn read_current_working_directory(&self) -> Result<Option<WorkingDirectoryId>> {
        let path = self.base_dir.current_working_directory_symlink_path();
        match std::fs::read_link(&path) {
            Ok(val) => {
                let s = val
                    .to_str()
                    .ok_or_else(|| anyhow!("missing symlink target in {path:?}"))?;
                let id = WorkingDirectoryId::from_prefixless_str(s)?;
                Ok(Some(id))
            }
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(e).map_err(ctx!("reading symlink {path:?}")),
            },
        }
    }

    pub fn read_working_directory_status(
        &self,
        id: WorkingDirectoryId,
    ) -> Result<WorkingDirectoryStatus> {
        let path = self.base_dir.path().append(id.to_directory_file_name());
        // XX partial copy paste from WorkingDirectory::open (ok not too much though)
        let status_path = WorkingDirectory::status_path_from_working_dir_path(&path)?;
        load_ron_file(&status_path)
    }
}

#[derive(Debug)]
pub struct WorkingDirectoryPool {
    opts: Arc<WorkingDirectoryPoolOpts>,
    remote_repository_url: GitUrl,
    // Actual basedir used (opts only has an Option!)
    base_dir: Arc<WorkingDirectoryPoolBaseDir>,
    next_id: u64,
    /// Contains working dirs with Status::Error, too, must be ignored
    /// when picking a dir!
    all_entries: BTreeMap<WorkingDirectoryId, WorkingDirectory>,
}

pub struct WorkingDirectoryPoolGuard<'pool> {
    // Option since it is also used via `to_non_mut`
    _lock: Option<OwningExclusiveFileLock<File>>,
    pool: &'pool WorkingDirectoryPool,
}

impl<'pool> WorkingDirectoryPoolGuard<'pool> {
    pub(crate) fn locked_working_directory_mut<'s: 'pool>(
        &'s self,
        wd: &'pool mut WorkingDirectory,
    ) -> WorkingDirectoryWithPoolLockMut<'pool> {
        WorkingDirectoryWithPoolLockMut { wd }
    }
}

pub struct WorkingDirectoryPoolGuardMut<'pool> {
    pub(crate) _lock: OwningExclusiveFileLock<File>,
    pub(crate) pool: &'pool mut WorkingDirectoryPool,
}

impl<'pool> WorkingDirectoryPoolGuardMut<'pool> {
    /// The mut guard can also do shared operations; XX todo: just
    /// Deref, ah, would be double use of deref?
    // NOTE: do *not* give 'pool life time to
    // WorkingDirectoryPoolGuard! Only as long as the lock is
    // guaranteed to be held!
    pub fn shared<'s: 'pool>(&'s self) -> WorkingDirectoryPoolGuard<'s> {
        WorkingDirectoryPoolGuard {
            pool: self.pool,
            // OK since self is guaranteed to outlive us
            _lock: None,
        }
    }

    pub fn locked_base_dir<'s>(&'s self) -> WorkingDirectoryPoolBaseDirLock<'s> {
        WorkingDirectoryPoolBaseDirLock {
            base_dir: &self.pool.base_dir,
            // OK since self is guaranteed to outlive us
            _lock: None,
        }
    }
}

pub struct WorkingDirectoryPoolAndLock(WorkingDirectoryPool, Option<OwningExclusiveFileLock<File>>);

impl WorkingDirectoryPoolAndLock {
    /// Take out the lock/guard; can only be done once
    pub fn take_guard<'t>(&'t mut self) -> Option<WorkingDirectoryPoolGuard<'t>> {
        Some(WorkingDirectoryPoolGuard {
            _lock: Some(self.1.take()?),
            pool: &mut self.0,
        })
    }

    /// Drop the lock and get the bare pool
    pub fn into_inner(self) -> WorkingDirectoryPool {
        self.0
    }
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingError {
    /// An Option since working directory pools are also used for
    /// things that are not benchmark runs
    benchmarking_job_parameters: Option<BenchmarkingJobParameters>,
    context: String,
    error: String,
}

impl WorkingDirectoryPool {
    /// Get exclusive lock, but sharing self
    pub fn lock<'t>(&'t self, locker: &str) -> Result<WorkingDirectoryPoolGuard<'t>> {
        let _lock = Some(self.base_dir.get_lock(locker)?);
        Ok(WorkingDirectoryPoolGuard { _lock, pool: self })
    }

    /// Get exclusive lock, for exclusive access to self
    pub fn lock_mut<'t>(&'t mut self, locker: &str) -> Result<WorkingDirectoryPoolGuardMut<'t>> {
        let _lock = self.base_dir.get_lock(locker)?;
        Ok(WorkingDirectoryPoolGuardMut { _lock, pool: self })
    }

    pub fn open(
        // XX why do we have working directory pool base dir twice,
        // via opts and base_dir? Just because of some weird
        // defaulting logic? I.e. the one in `opts` has to be ignored?
        opts: Arc<WorkingDirectoryPoolOpts>,
        base_dir: Arc<WorkingDirectoryPoolBaseDir>,
        remote_repository_url: GitUrl,
        create_dir_if_not_exists: bool,
    ) -> Result<WorkingDirectoryPoolAndLock> {
        if create_dir_if_not_exists {
            io_utils::div::create_dir_if_not_exists(base_dir.path(), "working pool directory")?;
        }

        // Need to have exclusive access while, at least, reading ron
        // files
        let lock = base_dir.get_lock("WorkingDirectoryPool::open")?;

        let mut next_id: u64 = 0;

        // To tell WorkingDirectory::open that we do have the lock we
        // need to make a guard, and for that we need a slf already,
        // thus make it early with false `all_entries` and `next_id`
        // entries.
        let mut slf = Self {
            opts,
            remote_repository_url,
            base_dir,
            next_id,
            all_entries: Default::default(),
        };

        let mut guard = WorkingDirectoryPoolGuard {
            _lock: Some(lock),
            pool: &mut slf,
        };

        let all_entries: BTreeMap<WorkingDirectoryId, WorkingDirectory> =
            std::fs::read_dir(guard.pool.base_dir.path())
                .map_err(ctx!(
                    "opening working pool directory {:?}",
                    guard.pool.base_dir.path()
                ))?
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
                        let wd = WorkingDirectory::open(path, &mut guard)?;
                        Ok(Some((id, wd)))
                    },
                )
                .filter_map(Result::transpose)
                .collect::<Result<_>>()
                .map_err(ctx!(
                    "reading contents of working pool directory {:?}",
                    guard.pool.base_dir.path()
                ))?;

        // Let go of the guard (so that we can mutate slf and later
        // return it), but keep the lock
        let lock = guard._lock.take().expect("we put it there above");

        slf.all_entries = all_entries;
        slf.next_id = next_id;

        info!(
            "opened directory pool {:?} with next_id {next_id}, len {}/{}",
            slf.base_dir,
            slf.active_len(),
            slf.capacity()
        );
        debug!("{slf:#?}");

        Ok(WorkingDirectoryPoolAndLock(slf, Some(lock)))
    }

    /// Also see the method on `WorkingDirectoryPoolGuard`!
    pub fn get_working_directory(
        &self,
        working_directory_id: WorkingDirectoryId,
    ) -> Option<&WorkingDirectory> {
        self.all_entries.get(&working_directory_id)
    }

    /// Also see the method on `WorkingDirectoryPoolGuard`!
    pub fn get_working_directory_mut(
        &mut self,
        working_directory_id: WorkingDirectoryId,
    ) -> Option<&mut WorkingDirectory> {
        self.all_entries.get_mut(&working_directory_id)
    }

    pub fn base_dir(&self) -> &WorkingDirectoryPoolBaseDir {
        &self.base_dir
    }

    /// The value from the configuration as `usize`. Guaranteed to be
    /// at least 1.
    pub fn capacity(&self) -> usize {
        self.opts.capacity.get().into()
    }

    pub fn git_url(&self) -> &GitUrl {
        &self.remote_repository_url
    }

    /// This includes working dirs with errors, that (normally) must
    /// be left aside and not used for processing!  The returned
    /// entries are sorted by `WorkingDirectoryId`
    pub fn all_entries(&self) -> impl Iterator<Item = (WorkingDirectoryId, &WorkingDirectory)> {
        self.all_entries.iter().map(|(id, wd)| (*id, wd))
    }

    pub fn all_entries_mut(
        &mut self,
    ) -> impl Iterator<Item = (WorkingDirectoryId, &mut WorkingDirectory)> {
        self.all_entries.iter_mut().map(|(id, wd)| (*id, wd))
    }

    /// The entries that can be used for processing. The returned
    /// entries are sorted by `WorkingDirectoryId`
    pub fn active_entries(&self) -> impl Iterator<Item = (WorkingDirectoryId, &WorkingDirectory)> {
        self.all_entries()
            .filter(|(_, wd)| wd.working_directory_status.status.can_be_used_for_jobs())
    }

    pub fn active_entries_mut(
        &mut self,
    ) -> impl Iterator<Item = (WorkingDirectoryId, &mut WorkingDirectory)> {
        self.all_entries_mut()
            .filter(|(_, wd)| wd.working_directory_status.status.can_be_used_for_jobs())
    }

    /// The number of entries that are not of Status::Error
    pub fn active_len(&self) -> usize {
        // Could optimize to not scan, but would want an abstraction
        // around the entries for that, don't do now.
        self.active_entries().count()
    }

    ///  Runs the given action on the requested working directory with
    ///  the pool lock; the lock allows to use working directory
    ///  actions that require the lock, but it's important to release
    ///  the lock as soon as possible via `into_inner()` (giving the
    ///  bare working directory, which can still be used for methods
    ///  that don't require the lock), so that e.g. `evobench-run wd`
    ///  actions don't block for the whole duration of an action
    ///  (i.e. a whole benchmarking run)!  If the action returns with
    ///  an error, stores it as metadata with the directory and
    ///  changes the working directory to status `Error`. Returns an
    ///  error if a working directory with the given id doesn't
    ///  exist. The returned `WorkingDirectoryCleanupToken` must be
    ///  passed to `working_directory_cleanup`. NOTE: is getting the
    ///  lock internally (multiple times for short durations, but also
    ///  passes the lock to `action` as mentioned above).
    pub fn process_in_working_directory<'pool, T>(
        &'pool mut self,
        working_directory_id: WorkingDirectoryId,
        timestamp: &DateTimeWithOffset,
        action: impl FnOnce(WorkingDirectoryWithPoolMut) -> Result<T>,
        benchmarking_job_parameters: Option<&BenchmarkingJobParameters>,
        context: &str,
        have_other_jobs_for_same_commit: Option<&dyn Fn() -> bool>,
    ) -> Result<(T, WorkingDirectoryCleanupToken)> {
        let mut guard =
            self.lock_mut("WorkingDirectoryPool.process_in_working_directory for action")?;

        guard.set_current_working_directory(working_directory_id)?;

        let mut wd = guard
            .get_working_directory_mut(working_directory_id)
            // Can't just .expect here because the use cases seem too
            // complex (concurrency means that a working directory
            // very well might disappear), thus:
            .ok_or_else(|| anyhow!("working directory id must still exist"))?;

        if !wd.working_directory_status.status.can_be_used_for_jobs() {
            bail!(
                "working directory {working_directory_id} is set aside (in '{}' state)",
                wd.working_directory_status.status
            )
        }

        wd.set_and_save_status(Status::Processing)?;

        info!(
            "process_working_directory {working_directory_id} \
             ({:?} for {context} at_{timestamp})...",
            benchmarking_job_parameters.map(BenchmarkingJobParameters::slow_hash)
        );

        match action(guard.into_get_working_directory_mut(working_directory_id)) {
            Ok(v) => {
                self.lock_mut("WorkingDirectoryPool.process_in_working_directory after action Ok")?
                    .get_working_directory_mut(working_directory_id)
                    .expect("we're not removing it in the mean time")
                    .set_and_save_status(Status::Finished)?;

                info!(
                    "process_working_directory {working_directory_id} \
                     ({:?} for {context} at_{timestamp}) succeeded.",
                    benchmarking_job_parameters.map(BenchmarkingJobParameters::slow_hash)
                );

                let wd = self
                    .get_working_directory(working_directory_id)
                    .expect("we're not removing it in the mean time");

                let needs_cleanup = wd.needs_cleanup(
                    self.opts.auto_clean.as_ref(),
                    have_other_jobs_for_same_commit,
                )?;
                let token = WorkingDirectoryCleanupToken {
                    working_directory_id,
                    needs_cleanup,
                    linear_token: Linear::new(false),
                };
                Ok((v, token))
            }
            Err(error) => {
                let mut lock = self.lock_mut(
                    "WorkingDirectoryPool.process_in_working_directory after action Err",
                )?;
                lock.get_working_directory_mut(working_directory_id)
                    .expect("we're not removing it in the mean time")
                    .set_and_save_status(Status::Error)?;

                let err = format!("{error:#?}");
                lock.save_processing_error(
                    working_directory_id,
                    ProcessingError {
                        benchmarking_job_parameters: benchmarking_job_parameters.cloned(),
                        context: context.to_string(),
                        error: err.clone(),
                    },
                    timestamp,
                )
                .map_err(ctx!("error storing the error {err}"))?;

                info!(
                    // Do not show error as it might be large; XX
                    // which is a mis-feature!
                    "process_working_directory {working_directory_id} \
                     ({:?} for {context} at_{timestamp}) failed.",
                    benchmarking_job_parameters.map(BenchmarkingJobParameters::slow_hash)
                );

                Err(error)
            }
        }
    }

    /// Possibly calls `delete_working_directory`, depending on what
    /// the token says. NOTE: takes the lock internally, only when
    /// needed.
    pub fn working_directory_cleanup(
        &mut self,
        cleanup: WorkingDirectoryCleanupToken,
    ) -> Result<()> {
        let WorkingDirectoryCleanupToken {
            linear_token,
            working_directory_id,
            needs_cleanup,
        } = cleanup;
        linear_token.bury();
        if needs_cleanup {
            let mut lock = self.lock_mut("WorkingDirectoryPool.working_directory_cleanup")?;
            lock.delete_working_directory(working_directory_id)?;
        }
        Ok(())
    }
}

impl<'pool> WorkingDirectoryPoolGuard<'pool> {
    /// There's also a method on `WorkingDirectoryPool`!
    pub fn get_working_directory<'guard: 'pool>(
        &'guard self,
        working_directory_id: WorkingDirectoryId,
    ) -> Option<WorkingDirectoryWithPoolLock<'guard>> {
        Some(WorkingDirectoryWithPoolLock {
            wd: self.pool.all_entries.get(&working_directory_id)?,
        })
    }
}

impl<'pool> WorkingDirectoryPoolGuardMut<'pool> {
    /// There's also a method on `WorkingDirectoryPool`!
    pub fn get_working_directory_mut<'guard>(
        &'guard mut self,
        working_directory_id: WorkingDirectoryId,
    ) -> Option<WorkingDirectoryWithPoolLockMut<'guard>> {
        Some(WorkingDirectoryWithPoolLockMut {
            wd: self.pool.all_entries.get_mut(&working_directory_id)?,
        })
    }

    /// Similar to `get_working_directory_mut` but transfer ownership
    /// of the guard into the result (does *not* unlock!).
    pub fn into_get_working_directory_mut(
        self,
        working_directory_id: WorkingDirectoryId,
    ) -> WorkingDirectoryWithPoolMut<'pool> {
        WorkingDirectoryWithPoolMut {
            guard: self,
            working_directory_id,
        }
    }

    /// Always gets a working directory, but doesn't check for any
    /// best fit. If none was cloned yet, that is done now.
    pub fn get_first(&mut self) -> Result<WorkingDirectoryId> {
        if let Some((key, _)) = self.pool.active_entries().next() {
            return Ok(key);
        }
        self.get_new()
    }

    /// This is *not* public as it is not checking whether there is
    /// capacity left for a new one!
    fn get_new(&mut self) -> Result<WorkingDirectoryId> {
        let id = self.next_id();
        debug!("get_new: using {id:?}");
        let dir = WorkingDirectory::clone_repo(
            self.pool.base_dir().path(),
            &id.to_directory_file_name(),
            self.pool.git_url(),
            &self.shared(),
        )?;
        self.pool.all_entries.insert(id, dir);
        Ok(id)
    }

    /// Save a processing error (not doing that to the status since
    /// that would get overwritten when changing it back to an active
    /// status). This method does *not* change the status of the
    /// working directory, that must be done separately.
    fn save_processing_error(
        &mut self,
        id: WorkingDirectoryId,
        processing_error: ProcessingError,
        timestamp: &DateTimeWithOffset,
    ) -> Result<()> {
        let error_file_path = self.pool.base_dir().path().append(format!(
            "{}.error_at_{timestamp}",
            id.to_directory_file_name()
        ));
        let processing_error_string = serde_yml::to_string(&processing_error)?;
        std::fs::write(&error_file_path, &processing_error_string)
            .map_err(ctx!("writing to {error_file_path:?}"))?;

        info!("saved processing error to {error_file_path:?}");

        Ok(())
    }

    /// Note: may leave behind a broken `current` symlink, but that's
    /// probably the way it should be?
    pub fn delete_working_directory(
        &mut self,
        working_directory_id: WorkingDirectoryId,
    ) -> Result<()> {
        let wd = self
            .pool
            .all_entries
            .get_mut(&working_directory_id)
            .ok_or_else(|| anyhow!("working directory id must still exist"))?;
        let path = wd.git_working_dir.working_dir_path_arc();
        info!("delete_working_directory: deleting directory {path:?}");
        self.pool.all_entries.remove(&working_directory_id);
        std::fs::remove_dir_all(&*path).map_err(ctx!("deleting directory {path:?}"))?;
        Ok(())
    }

    fn next_id(&mut self) -> WorkingDirectoryId {
        let id = self.pool.next_id;
        self.pool.next_id += 1;
        WorkingDirectoryId(id)
    }

    /// Ensure all *active* working directories have their commit field
    /// initialized
    fn init_active_commit_ids(&mut self) -> Result<()> {
        for (_, wd) in self.pool.active_entries_mut() {
            // SAFETY: It's OK to claim that the working dir has the
            // lock as we are a method of
            // `WorkingDirectoryPoolGuardMut` and locking currently
            // works on the whole pool.
            let mut wd = WorkingDirectoryWithPoolLockMut { wd };
            wd.commit()?;
        }
        Ok(())
    }

    /// Pick a working directory already checked out for the given
    /// commit, and if possible already built or even tested for
    /// it. Returns its id so that the right kind of fresh borrow can
    /// be done.
    fn try_get_fitting_working_directory_for(
        &mut self,
        run_parameters: &RunParameters,
        run_queues_data: &RunQueuesData,
    ) -> Result<Option<WorkingDirectoryId>> {
        // (todo?: is the working dir used last time for the same job
        // available? Maybe doesn't really matter any more though?)

        let commit: &GitHash = &run_parameters.commit_id;

        // Find one with the same commit. First ensure all commit
        // fields are set to avoid dealing with IO errors.
        self.init_active_commit_ids()?;
        if let Some((id, _dir)) = self
            .pool
            .active_entries_mut()
            .filter(|(_, wd)| wd.commit.as_ref().expect("initialized above") == commit)
            // Prefer one that proceeded further and is matching
            // closely: todo: store parameters for dir.
            .max_by_key(|(_, dir)| dir.working_directory_status.status)
        {
            info!("try_get_best_working_directory_for: found by commit {commit}");
            return Ok(Some(id));
        }

        // Find one that is *not* used by other jobs in the pipeline (i.e. obsolete),
        // and todo: similar parameters
        if let Some((id, _dir)) = self
            .pool
            .active_entries()
            .filter(|(_, dir)| {
                !run_queues_data
                    .have_job_with_commit_id(dir.commit.as_ref().expect("initialized above"))
            })
            .max_by_key(|(_, dir)| dir.working_directory_status.status)
        {
            info!("try_get_best_working_directory_for: found as obsolete");
            return Ok(Some(id));
        }

        Ok(None)
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
        if let Some(id) =
            self.try_get_fitting_working_directory_for(run_parameters, run_queues_data)?
        {
            info!("get_a_working_directory_for -> good old {id:?}");
            Ok(id)
        } else {
            if self.pool.active_len() < self.pool.capacity() {
                // allocate a new one
                let id = self.get_new()?;
                info!("get_a_working_directory_for -> new {id:?}");
                Ok(id)
            } else {
                // get the least-recently used one
                let id = self
                    .pool
                    .active_entries()
                    .min_by_key(|(_, entry)| entry.last_use)
                    .expect("capacity is guaranteed >= 1")
                    .0
                    .clone();
                info!("get_a_working_directory_for -> lru old {id:?}");
                Ok(id)
            }
        }
    }

    /// Remove the symlink to the currently used working
    /// directory. TODO: this is a mess, always forgetting; at least
    /// move to a compile time checked API? What was the purpose,
    /// really: sure, it was to put in some check that the dir was
    /// actually removed normally? But then that 'never' happens
    /// anyway? Do the removal statically (and for the case of errors
    /// preventing the removal, just always remove at runtime when
    /// setting it anew / do tmp-and-rename)?
    pub fn clear_current_working_directory(&self) -> Result<()> {
        let path = self.pool.base_dir.current_working_directory_symlink_path();
        if let Err(e) = std::fs::remove_file(&path) {
            match e.kind() {
                std::io::ErrorKind::NotFound => (),
                _ => Err(e).map_err(ctx!("removing symlink {path:?}"))?,
            }
        }
        Ok(())
    }

    /// Set the symlink to the currently used working directory. The
    /// old one must be removed beforehand via
    /// `clear_current_working_directory`.
    fn set_current_working_directory(&self, id: WorkingDirectoryId) -> Result<()> {
        let source_path = id.to_directory_file_name();
        let target_path = self.pool.base_dir.current_working_directory_symlink_path();
        std::os::unix::fs::symlink(&source_path, &target_path).map_err(ctx!(
            "creating symlink at {target_path:?} to {source_path:?}"
        ))
    }
}
