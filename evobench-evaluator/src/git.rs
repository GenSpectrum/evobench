use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Display},
    ops::Index,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, bail, Context, Result};
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

#[derive(Debug)]
pub struct EnrichedGitCommit<RefType> {
    pub commit: GitCommit<RefType>,
    /// The length of the longest parent chain (i.e. 0 for the initial
    /// commit)
    pub depth: usize,
}

impl GitCommit<Id<ToEnrichedCommit>> {
    /// Turn the `Id`s to commit hashes. Panics if the contained ids
    /// are not in `data`!
    pub fn with_ids_as_hashes(&self, data: &GitGraphData) -> GitCommit<GitHash> {
        let Self {
            commit_hash,
            author_time,
            committer_time,
            parents,
        } = self;

        let parents = parents
            .iter()
            .map(|parent| data[*parent].commit.commit_hash.clone())
            .collect();

        GitCommit {
            commit_hash: commit_hash.clone(),
            author_time: *author_time,
            committer_time: *committer_time,
            parents,
        }
    }
}

impl EnrichedGitCommit<Id<ToEnrichedCommit>> {
    /// Turn the `Id`s to commit hashes. Panics if the contained ids
    /// are not in `data`!
    pub fn with_ids_as_hashes(&self, data: &GitGraphData) -> EnrichedGitCommit<GitHash> {
        let Self { commit, depth } = self;
        let commit = commit.with_ids_as_hashes(data);
        EnrichedGitCommit {
            commit,
            depth: *depth,
        }
    }
}

// XX arbitrarily, was meant just for testing (same output as original
// git log output)
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

// XX arbitrarily
impl Display for EnrichedGitCommit<GitHash> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { commit, depth } = self;
        write!(f, "{depth}\t{commit}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id<Kind>(u32, Kind);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToEnrichedCommit;

// Consciously does not include a repository base path, to allow to
// collect graph data from multiple of them? Is this sensible or not
// really?
#[derive(Debug)]
pub struct GitGraphData {
    by_hash: HashMap<GitHash, Id<ToEnrichedCommit>>,
    ecommits: Vec<EnrichedGitCommit<Id<ToEnrichedCommit>>>,
}

#[derive(thiserror::Error, Debug)]
#[error("more than u32::max commits")]
pub struct MoreThanU32Commits;

impl Id<ToEnrichedCommit> {
    pub fn get(self, data: &GitGraphData) -> Option<&EnrichedGitCommit<Id<ToEnrichedCommit>>> {
        data.get(self)
    }
}

impl GitGraphData {
    fn new() -> Self {
        Self {
            by_hash: HashMap::new(),
            ecommits: Vec::new(),
        }
    }

    pub fn get(
        &self,
        id: Id<ToEnrichedCommit>,
    ) -> Option<&EnrichedGitCommit<Id<ToEnrichedCommit>>> {
        self.ecommits
            .get(usize::try_from(id.0).expect("at least 32 bit platform"))
    }

    pub fn get_by_hash(&self, h: &GitHash) -> Option<Id<ToEnrichedCommit>> {
        self.by_hash.get(h).copied()
    }

    // Does not check whether commit is already contained! But *does*
    // check that the parent ids are in range (they are referenced to
    // calculate the depth).
    pub fn unchecked_push(
        &mut self,
        commit: GitCommit<Id<ToEnrichedCommit>>,
    ) -> Result<Id<ToEnrichedCommit>, MoreThanU32Commits> {
        let commit_hash = commit.commit_hash.clone();

        let depth = commit
            .parents
            .iter()
            .map(|parent_id| self.get(*parent_id).expect("XXX").depth)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);

        let ecommit = EnrichedGitCommit { commit, depth };
        let id = self.ecommits.len();
        let id = Id(
            u32::try_from(id).map_err(|_| MoreThanU32Commits)?,
            ToEnrichedCommit,
        );
        self.by_hash.insert(commit_hash, id);
        self.ecommits.push(ecommit);
        Ok(id)
    }

    pub fn history_as_btreeset_from(
        &self,
        mut id: Id<ToEnrichedCommit>,
    ) -> BTreeSet<Id<ToEnrichedCommit>> {
        let mut ids = BTreeSet::new();
        let mut stack_of_commits_to_follow = Vec::new();
        loop {
            if ids.insert(id) {
                let ecommit = &self[id];
                let mut parents = ecommit.commit.parents.iter();
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
        commits: &BTreeSet<Id<ToEnrichedCommit>>,
        mut by: impl FnMut(&EnrichedGitCommit<Id<ToEnrichedCommit>>) -> T,
    ) -> Vec<Id<ToEnrichedCommit>> {
        let mut vec: Vec<_> = commits.iter().copied().collect();
        vec.sort_by_key(|id| by(&self[*id]));
        vec
    }

    pub fn ids_as_commits<'s: 'ids, 'ids>(
        &'s self,
        ids: &'ids [Id<ToEnrichedCommit>],
    ) -> impl DoubleEndedIterator<Item = &'s EnrichedGitCommit<Id<ToEnrichedCommit>>>
           + ExactSizeIterator
           + use<'s, 'ids> {
        ids.iter().map(|id| &self[*id])
    }

    pub fn commits(&self) -> &[EnrichedGitCommit<Id<ToEnrichedCommit>>] {
        &self.ecommits
    }

    pub fn add_history_from_dir_ref(
        &mut self,
        in_directory: impl AsRef<Path>,
        entry_reference: &str,
    ) -> Result<GitReference> {
        let in_directory = in_directory.as_ref();
        // XX first check if entry_reference is already indexed? But
        // need name index, too, then. And should make directory part
        // of the context, here, really.
        let commits = git_log_commits(in_directory, entry_reference)?;
        if commits.is_empty() {
            bail!("invalid Git reference {entry_reference:?} in Git dir {in_directory:?}")
        }
        Ok(GitReference::from_commits(
            KString::from_ref(entry_reference),
            commits.iter().rev(),
            self,
        )?)
    }
}

impl Index<Id<ToEnrichedCommit>> for GitGraphData {
    type Output = EnrichedGitCommit<Id<ToEnrichedCommit>>;

    fn index(&self, index: Id<ToEnrichedCommit>) -> &Self::Output {
        &self.ecommits[usize::try_from(index.0).expect("usize must be at least 32 bit wide")]
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
pub struct GitReference {
    pub name: KString,
    pub commit_id: Id<ToEnrichedCommit>,
}

impl GitReference {
    /// `commits` must come in order of creation, i.e. parents must
    /// come before children, or this panics! Also panics if commits
    /// is empty!
    pub fn from_commits<'c>(
        entry_reference: KString,
        commits: impl Iterator<Item = &'c GitCommit<GitHash>>,
        graph_lock: &mut GitGraphData,
    ) -> Result<Self, MoreThanU32Commits> {
        let mut entry_commit_id: Option<Id<ToEnrichedCommit>> = None;
        for GitCommit {
            commit_hash,
            author_time,
            committer_time,
            parents,
        } in commits
        {
            debug!("processing commit {commit_hash}");
            if let Some(id) = graph_lock.get_by_hash(commit_hash) {
                debug!("already recorded {commit_hash} earlier, ignoring the rest");
                // But need to continue, as there can be other merged
                // branches that come up later in the git log output.
                entry_commit_id = Some(id);
            } else {
                let commit = GitCommit {
                    commit_hash: commit_hash.clone(),
                    author_time: *author_time,
                    committer_time: *committer_time,
                    parents: parents
                        .iter()
                        .map(|parent| {
                            graph_lock.get_by_hash(parent).unwrap_or_else(|| {
                                panic!(
                                    "can't find parent {parent} of commit {commit_hash} -- \
                                     need commits with the oldest first!"
                                )
                            })
                        })
                        .collect(),
                };
                entry_commit_id = Some(graph_lock.unchecked_push(commit)?);
            }
        }
        Ok(GitReference {
            name: entry_reference,
            commit_id: entry_commit_id.expect("to be given non-empty commits iterator"),
        })
    }
}

/// Returns the commits with the newest one first. Careful:
/// `GitHistory::from_commits` expects them in the reverse order of
/// this one.  This returns a Vec (for lifetime reasons but also)
/// because it needs to be reversed afterwards, but also because
/// following branched Git history (via git log) can find branch with
/// known commits at some point, but the other still needing
/// exploration. Would need to analyze the history on the go to know
/// if stopping is OK. Thus for now, just get the whole
/// history. Returns the empty vector if the given reference does not
/// resolve! You usually do not want to use this function directly,
/// but instead initialize a GitGraph, get the lock, then run
/// `add_history_from_dir_ref` on it, which then uses this function.
pub fn git_log_commits(
    in_directory: impl AsRef<Path>,
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
            let items: SmallVec<[_; 4]> = line.split(|b| *b == b',').collect();
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
    fn t_() -> Result<()> {
        let git_in_cwd = AsRef::<Path>::as_ref("../.git");
        if !git_in_cwd.is_dir() {
            eprintln!("git.rs: not tested due to not being in a Git repository");
            return Ok(());
        }
        let graph = GitGraph::new();
        let mut graph_lock = graph.lock();

        let refs = [
            "9fd0ad621328a11a984aaa0700d54d05af6a899a",
            "f73da5abcc389db7754715a9fecadb478ecfbc16",
            "d165351476db2d65c3efcda89b4a99decede3784",
        ]
        .map(|name| {
            graph_lock
                .add_history_from_dir_ref(git_in_cwd, name)
                .map_err(|e| e.to_string())
        });

        dbg!(&refs);

        assert_eq!(
            refs[0].as_ref().unwrap().name.as_ref(),
            "9fd0ad621328a11a984aaa0700d54d05af6a899a"
        );

        let depths = refs.map(|r| r.unwrap().commit_id.get(&graph_lock).unwrap().depth);
        assert_eq!(depths, [159, 163, 161]);

        Ok(())
    }
}
