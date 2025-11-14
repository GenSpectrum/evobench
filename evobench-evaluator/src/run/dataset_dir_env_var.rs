use std::path::{Path, PathBuf};

use anyhow::Result;
use run_git::git::GitWorkingDir;

use crate::{git::GitHash, key::CustomParameters, run::versioned_dataset_dir::VersionedDatasetDir};

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
/// parameter values are provided.
pub fn dataset_dir_for(
    // If those two are given:
    versioned_datasets_base_dir: Option<&Path>,
    custom_parameters: &CustomParameters,
    // then calculate the result using them and those:
    versioned_dataset_dir: &VersionedDatasetDir,
    git_working_dir: &GitWorkingDir,
    commit_id: &GitHash,
) -> Result<Option<PathBuf>> {
    let versioned_datasets_base_dir = try_ok!(versioned_datasets_base_dir);

    let dataset_name = try_ok!(custom_parameters
        .btree_map()
        .get(&"DATASET".parse().expect("fits requirements")));
    // ^ XX hmm, check that the type of the custom env var
    // is a directory name?

    let vdirlock = versioned_dataset_dir.updated_git_graph(git_working_dir, commit_id)?;

    Ok(Some(vdirlock.dataset_dir_for_commit(
        versioned_datasets_base_dir,
        dataset_name.as_str(),
    )?))
}
