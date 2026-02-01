//! An abstraction for an *existing* directory, and one that should be
//! usable (i.e. is worth trying to use).

use std::{
    fmt::Display,
    fs::Permissions,
    ops::{Deref, DerefMut},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::{Result, anyhow, bail};
use run_git::{
    git::{GitResetMode, GitWorkingDir, git_clone},
    path_util::add_extension,
};
use serde::{Deserialize, Serialize};

use crate::{
    config_file::{load_ron_file, ron_to_file_pretty},
    ctx, debug,
    git::GitHash,
    git_ext::MoreGitWorkingDir,
    info,
    run::working_directory_pool::{
        WorkingDirectoryId, WorkingDirectoryPoolGuard, WorkingDirectoryPoolGuardMut,
    },
    serde::{date_and_time::DateTimeWithOffset, git_url::GitUrl},
    utillib::arc::CloneArc,
    warn,
};

/// The name of the default upstream; just Git's default name when
/// cloning, relying on that!
pub const REMOTE_NAME: &str = "origin";

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(rename = "WorkingDirectoryAutoClean")]
pub struct WorkingDirectoryAutoCleanOpts {
    /// The minimum age a working directory should reach before
    /// possibly being deleted, in days (recommended: 3)
    pub min_age_days: u16,

    /// The minimum number of jobs that should be run in a working
    /// directory before that is possibly being deleted (recommended:
    /// 80).
    pub min_num_runs: usize,

    /// If true, directories are not deleted when any job for the same
    /// commit id is in the queue.  (Directories are deleted when they
    /// reach both the `min_age_days` and `min_num_runs` numbers, and
    /// this is false, or the current job just ended and no others for
    /// the commit id exist.)
    pub wait_until_commit_done: bool,
}

const NO_OPTIONS: &[&str] = &[];

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Only checked out for that commit
    CheckedOut,
    /// Currently running in process_working_directory
    Processing,
    /// Project benchmarking gave error for that commit (means working dir is set aside)
    Error,
    /// Project benchmarking ran through for that commit
    Finished,
    /// Marked for examination, probably after an Error happened and
    /// it was decided to keep the directory around
    Examination,
}

impl Status {
    /// How well the dir is usable for a given commit id
    fn score(self) -> u32 {
        match self {
            Status::CheckedOut => 1,
            Status::Processing => 2,
            Status::Error => 3,
            Status::Finished => 4,
            Status::Examination => 5,
        }
    }

    /// Whether the daemon is allowed to use the dir
    pub fn can_be_used_for_jobs(self) -> bool {
        match self {
            Status::CheckedOut | Status::Processing | Status::Finished => true,
            Status::Error | Status::Examination => false,
        }
    }

    pub const MAX_STR_LEN: usize = 11;

    pub fn as_str(self) -> &'static str {
        match self {
            Status::CheckedOut => "checked-out",
            Status::Processing => "processing",
            Status::Error => "error",
            Status::Finished => "finished",
            Status::Examination => "examination",
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
        self.score().cmp(&other.score())
    }
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stored in `$n.status` files for each working directory
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkingDirectoryStatus {
    pub creation_timestamp: DateTimeWithOffset,
    pub num_runs: usize,
    pub status: Status,
}

impl WorkingDirectoryStatus {
    fn new() -> Self {
        Self {
            creation_timestamp: DateTimeWithOffset::now(),
            num_runs: 0,
            status: Status::CheckedOut,
        }
    }
}

// This is pretty much like WorkingDirectoryPool has a separate
// WorkingDirectoryPoolBaseDir, right? (Need to store Arc<PathBuf>
// since that's what `run-git` currently uses, should change that.)
/// A path to a working directory. Has methods that only need a path,
/// nothing else.
pub struct WorkingDirectoryPath(Arc<PathBuf>);

impl From<Arc<PathBuf>> for WorkingDirectoryPath {
    fn from(value: Arc<PathBuf>) -> Self {
        Self(value)
    }
}

impl From<WorkingDirectoryPath> for Arc<PathBuf> {
    fn from(value: WorkingDirectoryPath) -> Self {
        value.0
    }
}

impl From<WorkingDirectoryPath> for PathBuf {
    fn from(value: WorkingDirectoryPath) -> Self {
        match Arc::try_unwrap(value.0) {
            Ok(v) => v,
            Err(value) => value.as_path().to_owned(),
        }
    }
}

impl WorkingDirectoryPath {
    const STANDARD_LOG_EXTENSION_BASE: &str = "output_of_benchmarking_command_at_";
    pub fn standard_log_path_from_working_dir_path(
        path: &Path,
        timestamp: &DateTimeWithOffset,
    ) -> Result<PathBuf> {
        add_extension(
            path,
            format!("{}{timestamp}", Self::STANDARD_LOG_EXTENSION_BASE),
        )
        .ok_or_else(|| anyhow!("can't add extension to path {path:?}"))
    }
    pub fn standard_log_path(&self, timestamp: &DateTimeWithOffset) -> Result<PathBuf> {
        Self::standard_log_path_from_working_dir_path(&self.0, timestamp)
    }

    /// Originally thought `id` is a pool matter only, but now need it
    /// to filter for standard_log paths. Leaving id as string,
    /// though.)
    pub fn parent_path_and_id(&self) -> Result<(&Path, &str)> {
        let p = &self.0;
        let parent = p
            .parent()
            .ok_or_else(|| anyhow!("working directory path {p:?} doesn't have parent path"))?;
        let file_name = p
            .file_name()
            .ok_or_else(|| anyhow!("working directory path {p:?} doesn't have file_name"))?;
        let file_name = file_name.to_str().ok_or_else(|| {
            anyhow!("working directory path {p:?} does not have a file name in unicode")
        })?;
        Ok((parent, file_name))
    }

    /// All stdout log files that were written for this working
    /// directory: path including file name, just the
    /// timestamp. Sorted by timestamp, newest last.
    pub fn standard_log_paths(&self) -> Result<Vec<(PathBuf, String)>> {
        let (parent_path, id_str) = self.parent_path_and_id()?;
        let filename_prefix = format!("{id_str}.{}", Self::STANDARD_LOG_EXTENSION_BASE,);

        (|| -> Result<Vec<(PathBuf, String)>> {
            let mut paths = vec![];
            for item in std::fs::read_dir(&parent_path)? {
                let item = item?;
                if let Ok(file_name) = item.file_name().into_string() {
                    if let Some(timestamp) = file_name.strip_prefix(&filename_prefix) {
                        paths.push((item.path(), timestamp.to_owned()));
                    }
                }
            }
            paths.sort_by(|a, b| a.1.cmp(&b.1));
            Ok(paths)
        })()
        .map_err(ctx!(
            "opening working directory parent dir {parent_path:?} for reading"
        ))
    }

    pub fn last_standard_log_path(&self) -> Result<Option<(PathBuf, String)>> {
        Ok(self.standard_log_paths()?.pop())
    }

    pub fn noncached_commit(&self) -> Result<GitHash> {
        let git_working_dir = GitWorkingDir {
            working_dir_path: self.0.clone_arc(),
        };
        git_working_dir.get_head_commit_id()?.parse()
    }
}

#[derive(Debug)]
pub struct WorkingDirectory {
    pub git_working_dir: GitWorkingDir,
    /// Possibly initialized lazily via `commit()` accessor
    pub commit: Option<GitHash>,
    pub working_directory_status: WorkingDirectoryStatus,
    working_directory_status_needs_saving: bool,
    /// last use time: mtime of the .status file
    pub last_use: SystemTime,
}

pub struct WorkingDirectoryWithPoolLock<'guard> {
    // Don't make it plain `pub` as then could be constructed without
    // requiring going through the guard.
    pub(crate) wd: &'guard WorkingDirectory,
}

impl<'guard> WorkingDirectoryWithPoolLock<'guard> {
    pub fn into_inner(self) -> &'guard WorkingDirectory {
        self.wd
    }
}

impl<'guard> Deref for WorkingDirectoryWithPoolLock<'guard> {
    type Target = WorkingDirectory;

    fn deref(&self) -> &Self::Target {
        self.wd
    }
}

/// Does not own the lock! See `WorkingDirectoryWithPoolMut` for that.
pub struct WorkingDirectoryWithPoolLockMut<'guard> {
    // Don't make the field plain `pub` as then this could be
    // constructed without requiring going through the guard.
    pub(crate) wd: &'guard mut WorkingDirectory,
}

impl<'guard> Deref for WorkingDirectoryWithPoolLockMut<'guard> {
    type Target = WorkingDirectory;

    fn deref(&self) -> &Self::Target {
        self.wd
    }
}

impl<'guard> DerefMut for WorkingDirectoryWithPoolLockMut<'guard> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.wd
    }
}

/// Owns the lock
pub struct WorkingDirectoryWithPoolMut<'pool> {
    pub(crate) guard: WorkingDirectoryPoolGuardMut<'pool>,
    pub working_directory_id: WorkingDirectoryId,
}

impl<'pool> WorkingDirectoryWithPoolMut<'pool> {
    /// Get the working directory; does the lookup at this time, hence
    /// Option
    pub fn get<'s>(&'s mut self) -> Option<WorkingDirectoryWithPoolLockMut<'s>> {
        let Self {
            guard,
            working_directory_id,
        } = self;
        Some(WorkingDirectoryWithPoolLockMut {
            wd: guard
                .pool
                .get_working_directory_mut(*working_directory_id)?,
        })
    }

    /// Releases the lock. Retrieves the working directory at this
    /// time, hence Option.
    pub fn into_inner(self) -> Option<&'pool mut WorkingDirectory> {
        let Self {
            guard,
            working_directory_id,
        } = self;
        let WorkingDirectoryPoolGuardMut { _lock, pool } = guard;
        pool.get_working_directory_mut(working_directory_id)
    }
}

impl WorkingDirectory {
    pub fn status_path_from_working_dir_path(path: &Path) -> Result<PathBuf> {
        add_extension(&path, "status")
            .ok_or_else(|| anyhow!("can't add extension to path {path:?}"))
    }
    fn status_path(&self) -> Result<PathBuf> {
        Self::status_path_from_working_dir_path(self.git_working_dir.working_dir_path_ref())
    }

    /// To get access to methods that don't need a full
    /// WorkingDirectory, just its path.
    pub fn working_directory_path(&self) -> WorkingDirectoryPath {
        WorkingDirectoryPath(self.git_working_dir.working_dir_path_arc())
    }

    /// Open an existing working directory. Its default upstream
    /// (origin) is checked against `url` and changed to `url` if
    /// different; the idea here is that while the upstream URL may
    /// change (perhaps even semi-often), a configuration is always
    /// about the same target project, hence they share commits, hence
    /// deleting and re-cloning the working dir is not necessary or
    /// desired, just changing that url so that the newest changes can
    /// be retrieved.
    pub fn open<'pool>(
        path: PathBuf,
        url: &GitUrl,
        guard: &WorkingDirectoryPoolGuard<'pool>,
    ) -> Result<Self> {
        // let quiet = false;

        let working_directory_status_needs_saving;

        let status_path = Self::status_path_from_working_dir_path(&path)?;
        let (mtime, working_directory_status);
        match status_path.metadata() {
            Ok(metadata) => {
                mtime = metadata.modified()?;
                working_directory_status = load_ron_file(&status_path)?;
                working_directory_status_needs_saving = false;
            }
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        info!(
                            "note: missing working directory status file {status_path:?}, \
                             creating from defaults"
                        );
                        mtime = SystemTime::now();
                        working_directory_status = WorkingDirectoryStatus::new();
                        working_directory_status_needs_saving = true;
                    }
                    _ => {
                        return Err(e).map_err(ctx!(
                            "checking working directory status file path {status_path:?}"
                        ));
                    }
                };
            }
        }

        let git_working_dir = GitWorkingDir::from(path);
        let path = git_working_dir.working_dir_path_ref();

        // XX What was the idea here, just ensure that the directory
        // exists? But with the trailing `;`, seems like a left-over.
        {
            std::fs::metadata(path)
                .map_err(ctx!("WorkingDirectory::open({path:?})"))?
                .modified()?
        };

        // Check that the url is the same
        {
            let current_url = git_working_dir.get_url(REMOTE_NAME)?;
            if current_url != url.as_str() {
                warn!(
                    "the working directory at {path:?} has an {REMOTE_NAME:?} url != {url:?}: \
                     {current_url:?} -- setting it to the expected value"
                );
                git_working_dir.set_url(REMOTE_NAME, url)?;
            }
        }

        let status = working_directory_status.status;

        let mut slf = Self {
            git_working_dir,
            commit: None,
            working_directory_status,
            working_directory_status_needs_saving,
            last_use: mtime,
        };
        let mut slf_lck = guard.locked_working_directory_mut(&mut slf);
        // XX chaos: Do not change the status if it already
        // exists. Does this even work?
        slf_lck.set_and_save_status(status)?;
        Ok(slf)
    }

    pub fn clone_repo<'pool>(
        base_dir: &Path,
        dir_file_name: &str,
        url: &GitUrl,
        guard: &WorkingDirectoryPoolGuard<'pool>,
    ) -> Result<Self> {
        let quiet = false;
        let git_working_dir = git_clone(&base_dir, [], url.as_str(), dir_file_name, quiet)?;
        let commit: GitHash = git_working_dir.get_head_commit_id()?.parse()?;
        let status = WorkingDirectoryStatus::new();
        let mtime = status.creation_timestamp.to_systemtime();
        info!("clone_repo({base_dir:?}, {dir_file_name:?}, {url}) succeeded");
        let mut slf = Self {
            git_working_dir,
            commit: Some(commit),
            working_directory_status: status,
            working_directory_status_needs_saving: true,
            last_use: mtime,
        };
        let mut slf_lck = guard.locked_working_directory_mut(&mut slf);
        slf_lck.set_and_save_status(Status::CheckedOut)?;
        Ok(slf)
    }

    pub fn needs_cleanup(
        &self,
        opts: Option<&WorkingDirectoryAutoCleanOpts>,
        have_other_jobs_for_same_commit: Option<&dyn Fn() -> bool>,
    ) -> Result<bool> {
        if let Some(WorkingDirectoryAutoCleanOpts {
            min_age_days,
            min_num_runs,
            wait_until_commit_done,
        }) = opts
        {
            let is_old_enough: bool = {
                let min_age_days: u64 = (*min_age_days).into();
                let min_age = Duration::from_secs(24 * 3600 * min_age_days);
                let now = SystemTime::now();
                let creation_time: SystemTime = self
                    .working_directory_status
                    .creation_timestamp
                    .to_systemtime();
                let age = now.duration_since(creation_time).map_err(ctx!(
                    "calculating age for working directory {:?}",
                    self.git_working_dir.working_dir_path_ref()
                ))?;
                age >= min_age
            };
            let is_used_enough: bool = self.working_directory_status.num_runs >= *min_num_runs;
            Ok(is_old_enough
                && is_used_enough
                && ((!*wait_until_commit_done) || {
                    if let Some(have_other_jobs_for_same_commit) = have_other_jobs_for_same_commit {
                        have_other_jobs_for_same_commit()
                    } else {
                        // Could actually short-cut the calls from
                        // polling_pool.rs to false here. But making those
                        // configurable may still be good.
                        true
                    }
                }))
        } else {
            info!(
                "never cleaning up working directories since there is no \
                 `auto_clean` configuration"
            );
            Ok(false)
        }
    }

    /// Unconditionally run `git fetch --tags` in the working dir. Is
    /// called by `checkout` as needed. If `commit_id` is given, it is
    /// fetched explicitly
    pub fn fetch(&self, commit_id: Option<&GitHash>) -> Result<FetchedTags> {
        let git_working_dir = &self.git_working_dir;

        // Fetching tags in case `dataset_dir_for_commit` is
        // used.
        let fetch_all_tags = true;

        let tmp;
        let references = if let Some(commit_id) = commit_id {
            tmp = [commit_id.to_reference()];
            tmp.as_slice()
        } else {
            &[]
        };

        // Note: this does not update branches, right? But
        // branch names should never be used for anything, OK? XX
        // document?  or make the method fetch branches
        git_working_dir.fetch_references(REMOTE_NAME, fetch_all_tags, references, true)?;
        info!(
            "checkout({:?}, {commit_id:?}): ran fetch_references",
            git_working_dir.working_dir_path_ref()
        );

        Ok(FetchedTags::Yes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum FetchedTags {
    No,
    Yes,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FetchTags {
    WhenMissingCommit,
    Always,
}

impl<'guard> WorkingDirectoryWithPoolLockMut<'guard> {
    /// Retrieve commit via git if not already cached
    pub fn commit(&mut self) -> Result<&GitHash> {
        if self.commit.is_some() {
            Ok(self.commit.as_ref().expect("just checked"))
        } else {
            let commit = self.working_directory_path().noncached_commit()?;
            debug!(
                "set commit field of entry for WorkingDirectory {:?} to {commit}",
                self.wd.git_working_dir.working_dir_path_ref()
            );
            self.commit = Some(commit);
            Ok(self.commit.as_ref().expect("just set"))
        }
    }

    /// Checks and is a no-op if already on the commit.
    pub fn checkout(&mut self, commit: GitHash, fetch_tags: FetchTags) -> Result<FetchedTags> {
        let commit_str = commit.to_string();
        let quiet = false;
        let current_commit = self.wd.git_working_dir.get_head_commit_id()?;

        let fetch_tags_always = match fetch_tags {
            FetchTags::WhenMissingCommit => false,
            FetchTags::Always => true,
        };

        let ran_fetch;
        if current_commit == commit_str {
            if self.commit()? != &commit {
                bail!("consistency failure: dir on disk has different commit id from obj")
            }
            if fetch_tags_always {
                ran_fetch = self.fetch(Some(&commit))?;
            } else {
                ran_fetch = FetchedTags::No;
            }
        } else {
            let git_working_dir = &self.wd.git_working_dir;

            if (!fetch_tags_always) && git_working_dir.contains_reference(&commit_str)? {
                ran_fetch = FetchedTags::No;
            } else {
                ran_fetch = self.fetch(Some(&commit))?;
            }

            // First stash, merge --abort, cherry-pick --abort, and all
            // that jazz? No, have such a dir just go set aside with error
            // for manual fixing/removal.
            git_working_dir.git_reset(GitResetMode::Hard, NO_OPTIONS, &commit_str, quiet)?;
            info!(
                "checkout({:?}, {commit}): ran git reset --hard",
                self.wd.git_working_dir.working_dir_path_ref()
            );
            self.wd.commit = Some(commit);
            self.set_and_save_status(Status::CheckedOut)?;
        }
        Ok(ran_fetch)
    }

    /// Set status to `status`. Also increments the run count if the
    /// status changed to Status::Processing, and (re-)saves
    /// `$n.status` file if needed.
    // (pool is locked (races?, when is
    // the lock taken?--XXX OH, actually there is no lock!), nobody
    // can change such a file, thus we do not have to re-check if it
    // was changed on disk)-- XX what about this comment?
    pub fn set_and_save_status(&mut self, status: Status) -> Result<()> {
        debug!(
            "{:?} set_and_save_status({status:?})",
            self.wd.git_working_dir
        );
        let old_status = self.wd.working_directory_status.status;
        self.wd.working_directory_status.status = status;
        let needs_saving;
        if old_status != status {
            needs_saving = true;
            if status == Status::Processing {
                self.wd.working_directory_status.num_runs += 1;
            }
        } else {
            needs_saving = self.wd.working_directory_status_needs_saving;
        }
        if needs_saving {
            let working_directory_status = &self.wd.working_directory_status;
            let path = self.wd.status_path()?;
            ron_to_file_pretty(working_directory_status, &path, false, None)?;
            if !working_directory_status.status.can_be_used_for_jobs() {
                // Mis-use executable bit to easily see error status files
                // in dir listings on the command line.
                std::fs::set_permissions(&path, Permissions::from_mode(0o755))
                    .map_err(ctx!("setting executable permission on file {path:?}"))?;
            }
            debug!(
                "{:?} set_and_save_status({status:?}): file saved",
                self.wd.git_working_dir
            );
        }
        self.wd.working_directory_status_needs_saving = false;
        Ok(())
    }
}
