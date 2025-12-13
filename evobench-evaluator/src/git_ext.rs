//! Extension trait for `GitWorkingDir` from the `run-git` crate

use anyhow::{bail, Result};
use run_git::git::GitWorkingDir;

use crate::serde::git_url::GitUrl;

pub trait MoreGitWorkingDir {
    fn get_url(&self, remote_name: &str) -> Result<String>;
    fn set_url(&self, remote_name: &str, url: &GitUrl) -> Result<()>;
}

impl MoreGitWorkingDir for GitWorkingDir {
    fn get_url(&self, remote_name: &str) -> Result<String> {
        self.git_stdout_string_trimmed(&["remote", "get-url", remote_name])
    }

    fn set_url(&self, remote_name: &str, url: &GitUrl) -> Result<()> {
        if self.git(&["remote", "set-url", remote_name, url.as_str()], false)? {
            Ok(())
        } else {
            bail!(
                "got error status from `git remote set-url {remote_name:?} {:?}`",
                url.as_str()
            )
        }
    }
}
