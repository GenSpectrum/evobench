//! Extension for the `run-git` crate to retrieve git tags
//!
//! Probably to be moved there at some point.

use std::{collections::BTreeMap, str::FromStr, sync::Arc};

use anyhow::{Context, Result, anyhow};
use kstring::KString;
use run_git::git::GitWorkingDir;
use smallvec::{SmallVec, smallvec};

use crate::{git::GitHash, warn};

pub struct GitTag {
    pub name: KString,
    pub commit: Arc<GitHash>,
}

pub struct GitTags {
    tags: Vec<GitTag>,
    // To index in `tags`
    by_hash: BTreeMap<Arc<GitHash>, SmallVec<[u32; 1]>>,
}

impl GitTags {
    pub fn from_dir(working_dir: &GitWorkingDir) -> Result<Self> {
        let s = working_dir.git_stdout_string_trimmed(&[
            "tag", "--list", // -l
        ])?;
        let mut tags = Vec::new();
        let mut by_hash: BTreeMap<Arc<GitHash>, SmallVec<[u32; 1]>> = Default::default();
        for name in s.split("\n") {
            if let Some(commit) = working_dir
                .git_rev_parse(name, true)
                .with_context(|| anyhow!("resolving tag name {name:?}"))?
            {
                let commit = Arc::new(
                    GitHash::from_str(&commit)
                        .with_context(|| anyhow!("resolving tag name {name:?}"))?,
                );
                let id = u32::try_from(tags.len())
                    .context("getting tags, you appear to have more than u32::MAX tags")?;
                match by_hash.entry(commit.clone()) {
                    std::collections::btree_map::Entry::Vacant(vacant_entry) => {
                        vacant_entry.insert(smallvec![id]);
                    }
                    std::collections::btree_map::Entry::Occupied(mut occupied_entry) => {
                        occupied_entry.get_mut().push(id);
                    }
                }

                let name = KString::from_ref(name);
                tags.push(GitTag { name, commit });
            } else {
                // This *can* happen, there is a race between getting
                // the tag listing and another process deleting the
                // tags.
                warn!("could not resolve tag name {name:?} that we just got");
            }
        }
        Ok(GitTags { tags, by_hash })
    }

    /// Returns the empty set for unknown commit ids. Returns tag
    /// names in the order in which they appear in `git tag --list`.
    pub fn get_by_commit(&self, commit_id: &GitHash) -> impl ExactSizeIterator<Item = &str> {
        if let Some(items) = self.by_hash.get(commit_id) {
            &**items
        } else {
            &[]
        }
        .iter()
        .map(|i| &*self.tags[usize::try_from(*i).expect("usize expected to be >= u32")].name)
    }
}
