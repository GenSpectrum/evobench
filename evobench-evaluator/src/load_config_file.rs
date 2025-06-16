//! Generic config file loader

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;

/// Returns None if the file does not exist
pub fn try_load_json_file<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(json5::from_str(&s).with_context(|| {
            anyhow!("decoding JSON5 from config file {path:?}")
        })?)),
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => Ok(None),
            _ => bail!("loading config file from {path:?}: {e}"),
        },
    }
}

pub trait LoadConfigFile: Default + DeserializeOwned {
    fn default_config_path() -> Result<Option<PathBuf>>;

    /// If `path` is given, the file must exist or an error is
    /// returned. Otherwise, a default location is checked
    /// (`default_config_path`) and if exists, is loaded, if it
    /// doesn't exist, a `Default` instance is generated.
    fn load_config<P: AsRef<Path>>(path: Option<P>) -> Result<Self> {
        if let Some(path) = path {
            let path = path.as_ref();
            try_load_json_file(&path)?
                .ok_or_else(|| anyhow!("file with specified location {path:?} does not exist"))
        } else {
            if let Some(path) = Self::default_config_path()? {
                Ok(try_load_json_file(&path)?.unwrap_or_else(Self::default))
            } else {
                Ok(Self::default())
            }
        }
    }
}
