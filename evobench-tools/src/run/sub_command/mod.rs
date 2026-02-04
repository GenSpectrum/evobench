use std::sync::Arc;

use anyhow::Result;

use crate::run::{
    config::{RunConfig, RunConfigBundle},
    polling_pool::PollingPool,
    working_directory_pool::{
        WorkingDirectoryPool, WorkingDirectoryPoolAndLock, WorkingDirectoryPoolBaseDir,
        WorkingDirectoryPoolContext, WorkingDirectoryPoolOpts,
    },
};

pub mod insert;
pub mod list;
pub mod list_all;
pub mod wd;
pub mod wd_log;

// The reason to have these open_* functions here is just that making
// them methods on the pools spams those files with
// RunConfig/RunConfigBundle, and making them methods on RunConfig or
// RunConfigBundle spams those.

/// Takes `base_dir` since it needs to be constructed from what's in
/// `conf` and `GlobalAppStateDir`, which is done way up in main, so
/// we receive it from there, and we ignore the partially-duplicate
/// piece of data in `conf`.
pub fn open_working_directory_pool(
    conf: &RunConfig,
    base_dir: Arc<WorkingDirectoryPoolBaseDir>,
    omit_check: bool,
) -> Result<WorkingDirectoryPoolAndLock> {
    let create_dir_if_not_exists = true;

    let WorkingDirectoryPoolOpts {
        // `base_dir` is ignored since we take `base_dir` as argument
        // as described in the function doc
        base_dir: _,
        capacity,
        auto_clean,
    } = &*conf.working_directory_pool;

    WorkingDirectoryPool::open(
        WorkingDirectoryPoolContext {
            capacity: *capacity,
            auto_clean: auto_clean.clone(),
            remote_repository_url: conf.remote_repository.url.clone(),
            base_dir,
        },
        create_dir_if_not_exists,
        omit_check,
    )
}

pub fn open_polling_pool(config: &RunConfigBundle) -> Result<PollingPool> {
    PollingPool::open(
        config.run_config.remote_repository.url.clone(),
        config
            .global_app_state_dir
            .working_directory_for_polling_pool_base()?,
    )
}
