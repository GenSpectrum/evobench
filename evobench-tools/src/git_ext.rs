//! Extension trait for `GitWorkingDir` from the `run-git` crate

use std::str::FromStr;

use anyhow::{Result, bail};
use run_git::git::GitWorkingDir;

use crate::serde_types::{
    git_branch_name::GitBranchName, git_reference::GitReference, git_url::GitUrl,
};

pub trait MoreGitWorkingDir {
    fn get_url(&self, remote_name: &str) -> Result<String>;
    fn set_url(&self, remote_name: &str, url: &GitUrl) -> Result<()>;
    fn fetch_references<R: AsRef<GitReference>, Rs: AsRef<[R]>>(
        &self,
        remote_name: &str,
        tags: bool,
        references: Rs,
        quiet: bool,
    ) -> Result<()>;
    fn get_current_branch(&self) -> Result<Option<GitBranchName>>;
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

    /// Fetch branches (XXX does it do that?), optionally tags, and
    /// optional explicit commit ids (generally references?)
    fn fetch_references<R: AsRef<GitReference>, Rs: AsRef<[R]>>(
        &self,
        remote_name: &str,
        fetch_all_tags: bool,
        references: Rs,
        quiet: bool,
    ) -> Result<()> {
        let mut args = vec!["fetch", remote_name];
        if fetch_all_tags {
            args.push("--tags");
        }
        let rs = references.as_ref();
        for r in rs {
            let reference = r.as_ref();
            args.push(reference.as_ref());
        }

        // IIRC Git usually returns another exit code than 1 for
        // actual errors, right? But to be sure:
        if !self.git(&args, quiet)? {
            bail!("git {args:?} was not successful");
        }

        Ok(())
    }

    /// Get the name of the currently checked-out branch, if any
    /// (returns None if in detached HEAD state). TODO: just rename
    /// upstream method and change return type.
    fn get_current_branch(&self) -> Result<Option<GitBranchName>> {
        Ok(self.git_branch_show_current()?.map(|s| {
            GitBranchName::from_str(&s).expect("git always returns branch names for show-current")
        }))
    }
}
