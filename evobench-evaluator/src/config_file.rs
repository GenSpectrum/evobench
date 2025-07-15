//! Simple generic config file loader.

//! TODO: integrate `serde_path_to_error` crate

use std::{
    borrow::Borrow,
    borrow::Cow,
    fmt::Display,
    ops::Deref,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{anyhow, bail, Result};
use run_git::path_util::AppendToPath;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    ctx, info,
    json5_from_str::{json5_from_str, Json5FromStrError},
    path_util::add_extension,
    serde::proper_filename::ProperFilename,
    utillib::{
        home::{home_dir, HomeError},
        slice_or_box::SliceOrBox,
    },
};

pub fn ron_to_string_pretty<V: serde::Serialize>(value: &V) -> Result<String, ron::Error> {
    ron::Options::default().to_string_pretty(value, ron::ser::PrettyConfig::default())
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigBackend {
    Ron,
    Json5,
    Yaml,
    Hcl,
}

impl Display for ConfigBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.format_name())
    }
}

impl ConfigBackend {
    pub fn format_name(self) -> &'static str {
        match self {
            ConfigBackend::Ron => "RON",
            ConfigBackend::Json5 => "JSON5",
            ConfigBackend::Yaml => "YAML",
            ConfigBackend::Hcl => "HCL",
        }
    }

    pub fn load_config_file<T: DeserializeOwned>(self, path: &Path) -> Result<T> {
        let s =
            std::fs::read_to_string(&path).map_err(ctx!("loading config file from {path:?}"))?;
        match self {
            ConfigBackend::Ron => ron::from_str(&s)
                .map_err(|error| {
                    let ron::error::SpannedError {
                        code,
                        position: ron::error::Position { line, col },
                    } = error;
                    anyhow!("{code} at line:column {line}:{col}")
                })
                .map_err(ctx!("decoding RON from config file {path:?}")),
            ConfigBackend::Json5 => {
                // https://crates.io/crates/json5
                // https://crates.io/crates/serde_json5 <-- currently used, fork of json5
                // https://crates.io/crates/json5_nodes
                if false {
                    // Sadly this doesn't actually track paths,
                    // probably because the constructor already does
                    // the deserialisation.
                    let mut d = serde_json5::Deserializer::from_str(&s)
                        .map_err(Json5FromStrError)
                        .map_err(ctx!(
                            "decoding JSON5 from config file {path:?} -- step 1, too early?"
                        ))?;
                    // Also even loses location info this way.
                    serde_path_to_error::deserialize(&mut d)
                        .map_err(ctx!("decoding JSON5 from config file {path:?}"))
                } else {
                    json5_from_str(&s).map_err(ctx!("decoding JSON5 from config file {path:?}"))
                }
            }
            ConfigBackend::Yaml => {
                serde_yml::from_str(&s).map_err(ctx!("decoding YAML from config file {path:?}"))
            }
            ConfigBackend::Hcl => {
                hcl::from_str(&s).map_err(ctx!("decoding HCL from config file {path:?}"))
            }
        }
    }

    pub fn save_config_file<T: Serialize>(self, path: &Path, value: &T) -> Result<()> {
        let s = match self {
            ConfigBackend::Ron => ron_to_string_pretty(value)?,
            ConfigBackend::Json5 => {
                serde_json::to_string_pretty(value).map_err(ctx!("encoding config as JSON5"))?
            }
            ConfigBackend::Yaml => {
                serde_yml::to_string(value).map_err(ctx!("encoding config as YAML"))?
            }
            ConfigBackend::Hcl => hcl::to_string(value).map_err(ctx!("encoding config as HCL"))?,
        };
        std::fs::write(path, s).map_err(ctx!("writing config file to {path:?}"))
    }
}

pub const FILE_EXTENSIONS: &[(&str, ConfigBackend)] = &[
    ("ron", ConfigBackend::Ron),
    ("json5", ConfigBackend::Json5),
    ("json", ConfigBackend::Json5),
    ("yml", ConfigBackend::Yaml),
    ("yaml", ConfigBackend::Yaml),
    ("hcl", ConfigBackend::Hcl),
];

pub fn supported_formats() -> impl Iterator<Item = String> {
    FILE_EXTENSIONS
        .iter()
        .map(|(ext, backend)| format!(".{ext} ({backend})"))
}

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

pub enum ConfigDir {
    Etc,
    Home,
    Path(Cow<'static, Path>),
}

impl ConfigDir {
    pub fn to_path(&self) -> Result<Cow<Path>, &'static HomeError> {
        match self {
            ConfigDir::Etc => Ok(AsRef::<Path>::as_ref("/etc").into()),
            ConfigDir::Home => home_dir().map(Into::into),
            ConfigDir::Path(cow) => Ok(Cow::Borrowed(cow.borrow())),
        }
    }

    pub fn append_file_name(
        &self,
        file_name: &ProperFilename,
    ) -> Result<PathBuf, &'static HomeError> {
        match self {
            ConfigDir::Etc => Ok(self.to_path()?.append(file_name.as_ref())),
            ConfigDir::Home => {
                let dotted_file_name = format!(".{}", file_name.as_str());
                Ok(self.to_path()?.append(dotted_file_name))
            }
            ConfigDir::Path(cow) => Ok(cow.as_ref().append(file_name.as_ref())),
        }
    }
}

pub trait DefaultConfigPath: DeserializeOwned {
    /// `ConfigFile::load_config` tries this file name, together with
    /// a list of file name extensions, appended to the paths from
    /// `default_config_dirs()`, and one path is then expected to
    /// exist in a dir, if none the next is tried, or its `or_else`
    /// fallback is called. In the case of `ConfigDir::Home`, a dot is
    /// prepended to the file name.
    fn default_config_file_name_without_suffix() -> Result<Option<ProperFilename>>;

    fn default_config_dirs() -> SliceOrBox<'static, ConfigDir> {
        const V: &[ConfigDir] = &[ConfigDir::Home, ConfigDir::Etc];
        V.into()
    }
}

struct PathAndTrack {
    path: PathBuf,
    mtime: SystemTime,
}

/// Wrapper around a configuration type T that remembers where it was
/// loaded from and the modification time of the file, and can reload
/// a config if it changed.
pub struct ConfigFile<T> {
    config: T,
    path_and_track: Option<PathAndTrack>,
}

impl<T> Deref for ConfigFile<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl<T: DeserializeOwned + DefaultConfigPath> ConfigFile<T> {
    /// Check if the file that the config was loaded from has changed,
    /// if so, attempt to load it, if successful, overwrite self with
    /// the new value. Returns true if it did reload. Currently only
    /// checks the file that it was loaded from for changes; if this
    /// config was a default from `or_else`, no check is done at all.
    pub fn perhaps_reload_config<P: AsRef<Path>>(&mut self, provided_path: Option<P>) -> bool {
        if let Some(PathAndTrack { path, mtime }) = self.path_and_track.as_ref() {
            match std::fs::metadata(path) {
                Ok(s) => match s.modified() {
                    Ok(m) => {
                        if m == *mtime {
                            false
                        } else {
                            match Self::load_config(provided_path, |_| bail!("config missing")) {
                                Ok(val) => {
                                    *self = val;
                                    true
                                }
                                Err(_) => false,
                            }
                        }
                    }
                    Err(_) => false,
                },
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// If `path` is given, the file must exist or an error is
    /// returned. Otherwise, a default location is checked
    /// (`default_config_path_without_suffix`) and if a file with one
    /// of the fitting file name extensions exists, it is loaded,
    /// otherwise `or_else` is called with a message mentioning what
    /// was tried; it can issue an error or generate a default config
    /// value.
    pub fn load_config<P: AsRef<Path>>(
        path: Option<P>,
        or_else: impl FnOnce(String) -> Result<T>,
    ) -> Result<Self> {
        let load_config = |path: &Path, backend: ConfigBackend| {
            let config = backend.load_config_file(path)?;
            let mtime = std::fs::metadata(path)?.modified()?;
            Ok(Self {
                config,
                path_and_track: Some(PathAndTrack {
                    path: path.to_owned(),
                    mtime,
                }),
            })
        };

        if let Some(path) = path {
            let path = path.as_ref();
            let backend = backend_from_path(path)?;
            load_config(path, backend)
        } else {
            if let Some(file_name) = T::default_config_file_name_without_suffix()? {
                let mut default_paths_tried = Vec::new();
                for config_dir in T::default_config_dirs().iter() {
                    let path = config_dir.append_file_name(&file_name)?;
                    let path_and_backends: Vec<_> = FILE_EXTENSIONS
                        .into_iter()
                        .map(|(extension, backend)| {
                            let path = add_extension(&path, extension)
                                .ok_or_else(|| anyhow!("path is missing a file name: {path:?}"))?;
                            if path.exists() {
                                Ok(Some((path, backend)))
                            } else {
                                default_paths_tried.push(path);
                                Ok(None)
                            }
                        })
                        .filter_map(|x| x.transpose())
                        .collect::<Result<_>>()?;
                    match path_and_backends.len() {
                        0 => (),
                        1 => {
                            let (path, backend) = &path_and_backends[0];
                            info!("found config at {path:?}");
                            return load_config(&path, **backend);
                        }
                        _ => {
                            let paths = path_and_backends
                                .iter()
                                .map(|(path, _)| path)
                                .collect::<Vec<_>>();
                            bail!(
                                "multiple config file paths found, leading to ambiguity: {paths:?}"
                            )
                        }
                    }
                }
                let config = or_else(format!("tried the default paths: {default_paths_tried:?}"))?;
                Ok(Self {
                    config,
                    path_and_track: None,
                })
            } else {
                let config = or_else(format!(
                    "no path was given and there is no default \
                     config location for this type"
                ))?;
                Ok(Self {
                    config,
                    path_and_track: None,
                })
            }
        }
    }
}
