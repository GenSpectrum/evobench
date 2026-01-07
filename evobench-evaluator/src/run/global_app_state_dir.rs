use std::{fs::create_dir_all, path::PathBuf};

use anyhow::Result;
use run_git::path_util::AppendToPath;

use crate::{ctx, utillib::home::home_dir};

/// Relative path to directory from $HOME in which to keep state files
/// for the application.
const GLOBAL_APP_STATE_DIR_NAME: &str = ".evobench-jobs";

/// Representation of a directory below $HOME in which to keep state
/// for the installation. The full folder structure of that folder
/// should be represented via this type. Method calls to particular
/// subfolders create subfolder(s) as necessary.
pub struct GlobalAppStateDir {
    base_dir: PathBuf,
}

impl GlobalAppStateDir {
    /// Retrieves the $HOME value and creates the main subdir if
    /// necessary.
    pub fn new() -> Result<Self, anyhow::Error> {
        let home = home_dir()?;
        let base_dir = home.append(GLOBAL_APP_STATE_DIR_NAME);
        create_dir_all(&base_dir).map_err(ctx!("creating dir {base_dir:?}"))?;
        Ok(Self { base_dir })
    }

    fn subdir(&self, dir_name: &str) -> Result<PathBuf> {
        let dir = (&self.base_dir).append(dir_name);
        create_dir_all(&dir).map_err(ctx!("creating dir {dir:?}"))?;
        Ok(dir)
    }

    /// Directory used for:
    ///
    ///  * ensuring via flock on the directory that only one runner
    ///    instance is running,
    ///  * holding additional files specific for
    ///    that instance, e.g. `PollingSignals` files
    pub fn default_run_jobs_instance_basedir(&self) -> Result<PathBuf> {
        self.subdir("run_jobs_instance")
    }

    pub fn run_queues_basedir(&self) -> Result<PathBuf> {
        self.subdir("queues")
    }

    /// The pool of project clones which are used for building and benchmarking
    pub fn working_directory_pool_base(&self) -> Result<PathBuf> {
        self.subdir("working_directory_pool")
    }

    /// The pool of project clones (only 1, but the pool
    /// infrastructure is used to handle errors) for polling and for
    /// verifying commit ids on insertion
    pub fn working_directory_for_polling_pool_base(&self) -> Result<PathBuf> {
        self.subdir("polling_pool")
    }

    /// A KeyVal database of (run_parameters -> insertion time), for
    /// jobs already requested.
    pub fn already_inserted_base(&self) -> Result<PathBuf> {
        self.subdir("already_inserted")
    }
}
