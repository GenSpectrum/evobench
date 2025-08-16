use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Display},
    ops::Index,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result};
use itertools::Itertools;
use kstring::KString;
use smallvec::SmallVec;

pub use crate::serde::git_hash::GitHash;
use crate::{date_and_time::unixtime::Unixtime, debug};

#[derive(Debug)]
pub struct GitCommit<RefType> {
    pub commit_hash: GitHash,
    pub author_time: Unixtime,
    pub committer_time: Unixtime,
    pub parents: SmallVec<[RefType; 1]>,
}

impl GitCommit<Id<ToCommit>> {
    /// Panics if the contained ids are not in `data`
    pub fn to_hashes(&self, data: &GitGraphData) -> GitCommit<GitHash> {
        let Self {
            commit_hash,
            author_time,
            committer_time,
            parents,
        } = self;

        let parents = parents
            .iter()
            .map(|parent| data[*parent].commit_hash.clone())
            .collect();

        GitCommit {
            commit_hash: commit_hash.clone(),
            author_time: *author_time,
            committer_time: *committer_time,
            parents,
        }
    }
}

impl Display for GitCommit<GitHash> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            commit_hash,
            author_time,
            committer_time,
            parents,
        } = self;
        let a = author_time.0;
        let c = committer_time.0;
        let parents = parents.iter().map(|p| p.to_string()).join(" ");
        write!(f, "{commit_hash},{a},{c},{parents}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id<Kind>(u32, Kind);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToCommit;

#[derive(Debug)]
pub struct GitGraphData {
    by_hash: HashMap<GitHash, Id<ToCommit>>,
    commits: Vec<GitCommit<Id<ToCommit>>>,
}

impl GitGraphData {
    fn new() -> Self {
        Self {
            by_hash: HashMap::new(),
            commits: Vec::new(),
        }
    }

    pub fn by_hash(&self, h: &GitHash) -> Option<Id<ToCommit>> {
        self.by_hash.get(h).copied()
    }

    pub fn push(&mut self, commit: GitCommit<Id<ToCommit>>) -> Id<ToCommit> {
        let id = self.commits.len();
        let id = Id(
            // XX bad to panic here?
            u32::try_from(id).expect("fewer than u32::MAX commits"),
            ToCommit,
        );
        self.by_hash.insert(commit.commit_hash.clone(), id);
        self.commits.push(commit);
        id
    }

    pub fn history_from(&self, mut id: Id<ToCommit>) -> BTreeSet<Id<ToCommit>> {
        let mut ids = BTreeSet::new();
        let mut stack_of_commits_to_follow = Vec::new();
        loop {
            if ids.insert(id) {
                let commit = &self[id];
                let mut parents = commit.parents.iter();
                if let Some(first_parent) = parents.next() {
                    id = *first_parent;
                    for parent in parents {
                        stack_of_commits_to_follow.push(*parent);
                    }
                    continue;
                }
            }
            if let Some(to_follow) = stack_of_commits_to_follow.pop() {
                id = to_follow;
            } else {
                break;
            }
        }
        ids
    }

    pub fn sorted_by<T: Ord>(
        &self,
        commits: &BTreeSet<Id<ToCommit>>,
        mut by: impl FnMut(&GitCommit<Id<ToCommit>>) -> T,
    ) -> Vec<Id<ToCommit>> {
        let mut vec: Vec<_> = commits.iter().copied().collect();
        vec.sort_by_key(|id| by(&self[*id]));
        vec
    }

    pub fn ids_as_commits<'s: 'ids, 'ids>(
        &'s self,
        ids: &'ids [Id<ToCommit>],
    ) -> impl DoubleEndedIterator<Item = &'s GitCommit<Id<ToCommit>>> + ExactSizeIterator + use<'s, 'ids>
    {
        ids.iter().map(|id| &self[*id])
    }

    pub fn commits(&self) -> &[GitCommit<Id<ToCommit>>] {
        &self.commits
    }
}

impl Index<Id<ToCommit>> for GitGraphData {
    type Output = GitCommit<Id<ToCommit>>;

    fn index(&self, index: Id<ToCommit>) -> &Self::Output {
        &self.commits[usize::try_from(index.0).expect("at least u32 bit platform")]
    }
}

#[derive(Debug)]
pub struct GitGraph(Mutex<GitGraphData>);

impl GitGraph {
    pub fn new() -> Arc<Self> {
        Self(Mutex::new(GitGraphData::new())).into()
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, GitGraphData> {
        match self.0.lock() {
            Ok(l) => l,
            // Lock poisoning should never be able to hurt us (XX?),
            // thus just recover
            Err(l) => l.into_inner(),
        }
    }
}

#[derive(Debug)]
pub struct GitHistory {
    pub entry_reference: KString,
    pub entry_commit_id: Id<ToCommit>,
}

impl GitHistory {
    /// Important: `commits` must come in order of creation,
    /// i.e. parents must come before children, or this panics! Also
    /// panics if commits is empty!
    pub fn from_commits<'c>(
        entry_reference: KString,
        commits: impl Iterator<Item = &'c GitCommit<GitHash>>,
        graph_lock: &mut GitGraphData,
    ) -> Self {
        let mut entry_commit_id: Option<Id<ToCommit>> = None;
        for GitCommit {
            commit_hash,
            author_time,
            committer_time,
            parents,
        } in commits
        {
            debug!("processing commit {commit_hash}");
            if let Some(_id) = graph_lock.by_hash(commit_hash) {
                debug!("already recorded {commit_hash} earlier, nothing to do")
            } else {
                let commit = GitCommit {
                    commit_hash: commit_hash.clone(),
                    author_time: *author_time,
                    committer_time: *committer_time,
                    parents: parents
                        .iter()
                        .map(|parent| {
                            graph_lock.by_hash(parent).unwrap_or_else(|| {
                                panic!(
                                    "can't find parent {parent} of commit {commit_hash} -- \
                                     need commits with the oldest first!"
                                )
                            })
                        })
                        .collect(),
                };
                entry_commit_id = Some(graph_lock.push(commit));
            }
        }
        GitHistory {
            entry_reference,
            entry_commit_id: entry_commit_id.expect("to be given non-empty commits iterator"),
        }
    }
}

/// Returns the commits with the newest one first! You need to feed
/// them to `GitHistory::from_commits` in reverse order!
pub fn git_log_commits<D: AsRef<Path>>(
    in_directory: D,
    entry_reference: &str,
) -> Result<Vec<GitCommit<GitHash>>> {
    let in_directory = in_directory.as_ref();
    let mut c = Command::new("git");

    c.args(&[
        "log",
        {
            let commit_hash = "%H";
            let author_time = "%at";
            let committer_time = "%ct";
            let parent_hashes = "%P";
            &format!("--pretty={commit_hash},{author_time},{committer_time},{parent_hashes}")
        },
        entry_reference,
    ]);
    let str_from_bytes =
        |bs| std::str::from_utf8(bs).expect("git always gives ascii with given arguments");
    c.current_dir(in_directory);

    c.stdout(Stdio::piped());
    let output = c
        .output()
        .with_context(|| anyhow!("in directory {in_directory:?}"))?;
    output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| -> Result<GitCommit<GitHash>> {
            let items: Vec<_> = line.split(|b| *b == b',').collect();
            if let [commit_hash, author_time, committer_time, parents] = items.as_slice() {
                let commit_hash = GitHash::try_from(*commit_hash)?;
                let author_time = Unixtime(u64::from_str_radix(str_from_bytes(author_time), 10)?);
                let committer_time =
                    Unixtime(u64::from_str_radix(str_from_bytes(committer_time), 10)?);
                let parents: SmallVec<_> = parents
                    .split(|b| *b == b' ')
                    .into_iter()
                    .filter(|bs| !bs.is_empty())
                    .map(GitHash::try_from)
                    .collect::<Result<_>>()?;
                Ok(GitCommit {
                    commit_hash,
                    author_time,
                    committer_time,
                    parents,
                })
            } else {
                unreachable!("4 fields from git")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_() {}
}
