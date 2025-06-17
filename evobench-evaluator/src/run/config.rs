use std::{collections::BTreeMap, path::PathBuf};

use anyhow::Result;

use crate::{load_config_file::LoadConfigFile, path_util::AppendToPath, utillib::home::home_dir};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    /// If not given, `~/.evobench-run-queue/` is used
    run_queue_path: Option<PathBuf>,
    /// The key names (environment variable names) that are allowed
    /// (value `false`) or required (value `true`) for benchmarking
    /// the given project
    pub custom_parameters_required: BTreeMap<String, bool>,
}

impl RunConfig {
    pub fn run_queue_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.run_queue_path {
            Ok(path.into())
        } else {
            let home = home_dir()?;
            Ok(home.append(".evobench-run-queue"))
        }
    }
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            run_queue_path: None,
            custom_parameters_required: Default::default(),
        }
    }
}

impl LoadConfigFile for RunConfig {
    fn default_config_path_without_suffix() -> Result<Option<PathBuf>> {
        let home = home_dir()?;
        Ok(Some(home.append(".evobench-run")))
    }
}
