use std::{
    collections::HashMap,
    fmt::Debug,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{anyhow, Context, Result};
use kstring::KString;

pub use crate::serde::git_hash::GitHash;

#[derive(Debug)]
pub struct GitCommit {
    pub author_time: u64,
    pub parents: Vec<GitHash>,
}

#[derive(Debug)]
pub struct GitGraph {
    pub entry_reference: KString,
    pub entry_githash: Option<GitHash>,
    pub commits: HashMap<GitHash, GitCommit>,
}

impl GitGraph {
    pub fn new_dir_ref<D: AsRef<Path>>(in_directory: D, entry_reference: &str) -> Result<GitGraph> {
        let in_directory = in_directory.as_ref();
        let mut c = Command::new("git");
        c.args(&["log", "--pretty=%at,%H,%P"]);
        let str_from_bytes =
            |bs| std::str::from_utf8(bs).expect("git always gives ascii with given arguments");
        c.current_dir(in_directory);
        let mut commits = HashMap::new();
        c.stdout(Stdio::piped());
        let output = c
            .output()
            .with_context(|| anyhow!("in directory {in_directory:?}",))?;
        let mut entry_githash: Option<GitHash> = None;
        for line in output.stdout.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            let items: Vec<_> = line.split(|b| *b == b',').collect();
            if let [author_time, hash, parents] = items.as_slice() {
                let author_time = u64::from_str_radix(str_from_bytes(author_time), 10)?;
                let hash = GitHash::try_from(*hash)?;
                if entry_githash.is_none() {
                    entry_githash = Some(hash.clone());
                }
                let parents: Vec<_> = parents
                    .split(|b| *b == b' ')
                    .into_iter()
                    .filter(|bs| !bs.is_empty())
                    .map(GitHash::try_from)
                    .collect::<Result<_>>()?;
                let commit = GitCommit {
                    author_time,
                    parents,
                };
                commits.insert(hash, commit);
            } else {
                unreachable!("3 fields from git")
            }
        }

        Ok(Self {
            entry_reference: KString::from_ref(entry_reference),
            entry_githash,
            commits,
        })
    }

    pub fn get(&self, h: &GitHash) -> Option<&GitCommit> {
        self.commits.get(h)
    }
}
