use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use run_git::git::GitWorkingDir;

use crate::{
    git::GitHash,
    key::CustomParameters,
    run::{
        custom_parameter::CustomParameterType, versioned_dataset_dir::VersionedDatasetDir,
        working_directory::FetchedTags,
    },
};

macro_rules! try_ok {
    { $e:expr } =>  {
        match $e {
            Some(v) => v,
            None => return Ok(None)
        }
    }
}

/// Find the matching dataset if both the
/// `versioned_datasets_base_dir` config and the `DATASET` custom
/// parameter values are provided. Wants to be assured via
/// `fetched_tags` that `git fetch --tags` was run (see methods that
/// return a `FetchedTags`).
pub fn dataset_dir_for(
    // If those two are given:
    versioned_datasets_base_dir: Option<&Path>,
    custom_parameters: &CustomParameters,
    // then calculate the result using them and those:
    versioned_dataset_dir: &VersionedDatasetDir,
    git_working_dir: &GitWorkingDir,
    commit_id: &GitHash,
    fetched_tags: FetchedTags,
) -> Result<Option<PathBuf>> {
    if fetched_tags != FetchedTags::Yes {
        bail!("dataset_dir_for: require updated tags, but got {fetched_tags:?}")
    }

    let versioned_datasets_base_dir = try_ok!(versioned_datasets_base_dir);

    let key = "DATASET";
    let dataset_name = try_ok!(
        custom_parameters
            .btree_map()
            .get(&key.parse().expect("fits requirements"))
    );

    let ty = dataset_name.r#type();
    if ty != CustomParameterType::Dirname {
        bail!("custom parameter {key:?} is expected to be a Dirname, but is defined as {ty:?}")
    }

    let vdirlock = versioned_dataset_dir.updated_git_graph(git_working_dir, commit_id)?;

    Ok(Some(vdirlock.dataset_dir_for_commit(
        versioned_datasets_base_dir,
        dataset_name.as_str(),
    )?))
}
