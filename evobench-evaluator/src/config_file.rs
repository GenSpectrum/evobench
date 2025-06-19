//! Generic config file loader

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};

use crate::{json5_from_str::json5_from_str, path_util::add_extension};

#[derive(Debug, Clone, Copy)]
pub enum ConfigBackend {
    Json5,
    Yaml,
    Hcl,
}

impl ConfigBackend {
    pub fn load_config_file<T: DeserializeOwned>(self, path: &Path) -> Result<T> {
        let s = std::fs::read_to_string(&path)
            .with_context(|| anyhow!("loading config file from {path:?}"))?;
        match self {
            ConfigBackend::Json5 => json5_from_str(&s)
                .with_context(|| anyhow!("decoding JSON5 from config file {path:?}")),
            ConfigBackend::Yaml => serde_yml::from_str(&s)
                .with_context(|| anyhow!("decoding YAML from config file {path:?}")),
            ConfigBackend::Hcl => {
                hcl::from_str(&s).with_context(|| anyhow!("decoding HCL from config file {path:?}"))
            }
        }
    }

    pub fn save_config_file<T: Serialize>(self, path: &Path, value: &T) -> Result<()> {
        let s = match self {
            ConfigBackend::Json5 => {
                json5::to_string(value).with_context(|| anyhow!("encoding config as JSON5"))?
            }
            ConfigBackend::Yaml => {
                serde_yml::to_string(value).with_context(|| anyhow!("encoding config as YAML"))?
            }
            ConfigBackend::Hcl => {
                hcl::to_string(value).with_context(|| anyhow!("encoding config as HCL"))?
            }
        };
        std::fs::write(path, s).with_context(|| anyhow!("writing config file to {path:?}"))
    }
}

pub const FILE_EXTENSIONS: &[(&str, ConfigBackend)] = &[
    ("json5", ConfigBackend::Json5),
    ("json", ConfigBackend::Json5),
    ("yml", ConfigBackend::Yaml),
    ("yaml", ConfigBackend::Yaml),
    ("hcl", ConfigBackend::Hcl),
];

pub fn backend_from_path(path: &Path) -> Result<ConfigBackend> {
    if let Some(ext) = path.extension() {
        if let Some(ext) = ext.to_str() {
            if let Some((_, backend)) = FILE_EXTENSIONS.iter().find(|(e, _b)| *e == ext) {
                Ok(*backend)
            } else {
                bail!("given file path does have an unknown extension {ext:?}: {path:?}")
            }
        } else {
            bail!("given file path does have an extension that is not unicode: {path:?}")
        }
    } else {
        bail!(
            "given file path does not have an extension \
                     for determining the file type: {path:?}"
        )
    }
}

pub fn save_config_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let backend = backend_from_path(path)?;
    backend.save_config_file(path, value)
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
            let backend = backend_from_path(path)?;
            backend.load_config_file(path)
        } else {
            if let Some(path) = Self::default_config_path_without_suffix()? {
                let path_and_backends: Vec<_> = FILE_EXTENSIONS
                    .into_iter()
                    .map(|(extension, backend)| {
                        let path = add_extension(&path, extension)
                            .ok_or_else(|| anyhow!("path is missing a file name: {path:?}"))?;
                        if path.exists() {
                            Ok(Some((path, backend)))
                        } else {
                            Ok(None)
                        }
                    })
                    .filter_map(|x| x.transpose())
                    .collect::<Result<_>>()?;
                let paths = path_and_backends
                    .iter()
                    .map(|(path, _)| path)
                    .collect::<Vec<_>>();
                match path_and_backends.len() {
                    0 => (),
                    1 => {
                        let (path, backend) = &path_and_backends[0];
                        return backend.load_config_file(&path);
                    }
                    _ => {
                        bail!("multiple config file paths found, leading to ambiguity: {paths:?}")
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
