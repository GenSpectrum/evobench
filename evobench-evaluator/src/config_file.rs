//! Generic config file loader

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;

use crate::{json5_from_str::json5_from_str, path_util::add_extension};

/// Returns None if the file does not exist
pub fn try_load_json_file<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(json5_from_str(&s).with_context(|| {
            anyhow!("decoding JSON5 from config file {path:?}")
        })?)),
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => Ok(None),
            _ => bail!("loading config file from {path:?}: {e}"),
        },
    }
}

pub trait LoadConfigFile: DeserializeOwned {
    /// ".json5" and ".json" will be appended (and tried in order, but
    /// the chosen suffix has no effect on the parser)
    fn default_config_path_without_suffix() -> Result<Option<PathBuf>>;

    /// If `path` is given, the file must exist or an error is
    /// returned. Otherwise, a default location is checked
    /// (`default_config_path_without_suffix`) and if a file with one
    /// of the fitting file name extensions exists, it is loaded,
    /// otherwise `or_else` is called with a message mentioning what
    /// was tried; it can issue an error or generate a default config
    /// value.
    fn load_config<P: AsRef<Path>>(
        path: Option<P>,
        or_else: impl FnOnce(String) -> Result<Self>,
    ) -> Result<Self> {
        if let Some(path) = path {
            let path = path.as_ref();
            try_load_json_file(&path)?
                .ok_or_else(|| anyhow!("file with specified location {path:?} does not exist"))
        } else {
            if let Some(path) = Self::default_config_path_without_suffix()? {
                let paths: Vec<_> = vec!["json5", "json"]
                    .into_iter()
                    .map(|extension| {
                        add_extension(&path, extension)
                            .ok_or_else(|| anyhow!("path is missing a file name: {path:?}"))
                    })
                    .collect::<Result<_>>()?;

                for path in &paths {
                    if let Some(c) = try_load_json_file(path)? {
                        return Ok(c);
                    }
                }
                or_else(format!("tried the default paths: {paths:?}"))
            } else {
                or_else(format!(
                    "no path was given and there is no default \
                     config location for this type"
                ))
            }
        }
    }
}
