use std::path::PathBuf;

use anyhow::Result;

use crate::{load_config_file::LoadConfigFile, path_util::AppendToPath, utillib::home::home_dir};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RunConfig {
    /// If not given, `~/.evobench-run-queue/` is used
    run_queue_path: Option<PathBuf>,
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
        }
    }
}

impl LoadConfigFile for RunConfig {
    fn default_config_path() -> Result<Option<PathBuf>> {
        let home = home_dir()?;
        Ok(Some(home.append(".evobench-run.rs")))
    }
}
