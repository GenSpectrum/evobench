//! The directory structure for output files.

use std::{
    cell::OnceCell,
    collections::BTreeMap,
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Result, anyhow, bail};
use cj_path_util::path_util::AppendToPath;
use kstring::KString;

use crate::{
    clone, ctx,
    git::GitHash,
    key::{CustomParameters, ExtendPath, RunParameters, UncheckedCustomParameters},
    serde::{
        allowed_env_var::AllowedEnvVar, date_and_time::DateTimeWithOffset,
        proper_dirname::ProperDirname, proper_filename::ProperFilename,
    },
    utillib::{arc::CloneArc, into_arc_path::IntoArcPath, type_name_short::type_name_short},
};

pub trait ToPath {
    /// May be slightly costly on first run but then cached in those
    /// cases
    fn to_path(&self) -> &Arc<Path>;
}

pub trait SubDirs: ToPath {
    type Target;
    fn append_str(self: Arc<Self>, file_name: &str) -> Result<Self::Target>;

    /// Skips non-directory entries, but requires all directory entries to
    /// be convertible to `T`.
    fn sub_dirs(self: &Arc<Self>) -> Result<Vec<Self::Target>> {
        let dir_path = self.to_path();
        std::fs::read_dir(dir_path)
            .map_err(ctx!("opening dir {dir_path:?}"))?
            .map(|entry| -> Result<Option<Self::Target>> {
                let entry: std::fs::DirEntry = entry?;
                let ft = entry.file_type()?;
                if ft.is_dir() {
                    if let Some(file_name) = entry.file_name().to_str() {
                        Ok(Some(self.clone_arc().append_str(&file_name)?))
                    } else {
                        // silently ignore those paths, OK?
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            })
            .filter_map(|r| r.transpose())
            .collect::<Result<_, _>>()
            .map_err(ctx!(
                "getting {} listing for dir {dir_path:?}",
                type_name_short::<Self>()
            ))
    }
}

pub trait ReplaceBasePath {
    fn replace_base_path(&self, base_path: Arc<Path>) -> Self;
}

/// Parse a path's filename as T
fn parse_path_filename<T: FromStr>(path: &Path) -> Result<(T, &Path)>
where
    T::Err: Display,
{
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path is missing a file name"))?;
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("path is missing a parent dir"))?;
    if let Ok(file_name_str) = file_name.to_owned().into_string() {
        match T::from_str(&file_name_str) {
            Ok(v) => Ok((v, dir)),
            Err(e) => {
                bail!(
                    "dir name {file_name_str:?} in {dir:?} \
                     does not parse as {}: {e:#}",
                    type_name_short::<T>()
                )
            }
        }
    } else {
        let lossy1 = file_name.to_string_lossy();
        let lossy: &str = lossy1.as_ref();
        bail!("can't decode dir name to string: {lossy:?} in {dir:?}");
    }
}

/// Need to be able to handle unchecked custom parameters for parsing
/// since there's no config file for parsing a file system, or more to
/// the point, when the config changes, the file system must still be
/// readable, thus the representation must remain independent of the
/// config. But allow to represent both. Do this at runtime for
/// ~simplicity. (Maybe this should be moved to where the contained
/// types are defined, which maybe shouldn't all be in `key.rs`.)
#[derive(Debug, Clone)]
pub enum CheckedOrUncheckedCustomParameters {
    UncheckedCustomParameters(Arc<UncheckedCustomParameters>),
    CustomParameters(Arc<CustomParameters>),
}

impl CheckedOrUncheckedCustomParameters {
    fn extend_path(&self, path: PathBuf) -> PathBuf {
        match self {
            CheckedOrUncheckedCustomParameters::UncheckedCustomParameters(v) => v.extend_path(path),
            CheckedOrUncheckedCustomParameters::CustomParameters(v) => v.extend_path(path),
        }
    }
}

// --- The types ----------------------------------------------------------------

/// The dir representing all of a key except for the commit id
/// (i.e. custom parameters and target name--note that this is *not*
/// the same info as `RunParameters` contains!).
///
/// Note that it contains env vars that may *not* be checked against
/// the config. They are still guaranteed to follow the general
/// requirements for env var names (as per
/// `AllowedEnvVar<AllowableCustomEnvVar>::from_str`). Similarly,
/// `target_name` may not be checked against anything (other than
/// being a directory name).
#[derive(Debug)]
pub struct ParametersDir {
    base_path: Arc<Path>,
    target_name: ProperDirname,
    custom_parameters: CheckedOrUncheckedCustomParameters,
    path_cache: OnceCell<Arc<Path>>,
}

/// Dir representing all of the key, including commit id at the
/// end. I.e. one level below a `RunParametersDir`.
#[derive(Debug, Clone)]
pub struct KeyDir {
    parent: Arc<ParametersDir>,
    commit_id: GitHash,
    path_cache: OnceCell<Arc<Path>>,
}

/// Dir with the results for an individual benchmarking run. I.e. one
/// level below a `KeyDir`.
#[derive(Debug, Clone)]
pub struct RunDir {
    parent: Arc<KeyDir>,
    timestamp: DateTimeWithOffset,
    path_cache: OnceCell<Arc<Path>>,
}

// --- Their implementations ----------------------------------------------------

impl ToPath for ParametersDir {
    fn to_path(&self) -> &Arc<Path> {
        let Self {
            base_path,
            target_name,
            custom_parameters,
            path_cache,
        } = self;
        if path_cache.get().is_none() {
            let path = custom_parameters.extend_path(base_path.append(target_name.as_str()));
            _ = path_cache.set(path.into());
        }
        self.path_cache.get().unwrap()
    }
}

impl TryFrom<Arc<Path>> for ParametersDir {
    type Error = anyhow::Error;

    fn try_from(path: Arc<Path>) -> std::result::Result<Self, Self::Error> {
        let target_name;
        let custom_env_vars;
        let base_path;
        {
            let mut current_path = &*path;
            let mut current_vars = BTreeMap::new();
            loop {
                if let Some(dir_name) = current_path.file_name() {
                    let dir_name_str = dir_name.to_str().ok_or_else(|| {
                        anyhow!(
                            "directory segment can't be decoded as string: {:?} in {:?}",
                            dir_name.to_string_lossy().as_ref(),
                            path
                        )
                    })?;
                    if let Some((var_name, val)) = dir_name_str.split_once('=') {
                        let key = AllowedEnvVar::from_str(var_name)?;
                        let val = KString::from_ref(val);
                        current_vars.insert(key, val);

                        if let Some(parent) = current_path.parent() {
                            current_path = parent;
                        } else {
                            bail!(
                                "parsing {} {:?}: ran out of parent segments",
                                type_name_short::<Self>(),
                                path
                            );
                        }
                    } else {
                        target_name = ProperDirname::from_str(dir_name_str).map_err(|msg| {
                            anyhow!("not a proper directory name: {dir_name_str:?}: {msg}")
                        })?;
                        custom_env_vars = current_vars;
                        base_path = current_path.into();
                        break;
                    }
                }
            }
        }
        Ok(Self {
            base_path,
            target_name,
            custom_parameters: CheckedOrUncheckedCustomParameters::UncheckedCustomParameters(
                Arc::new(UncheckedCustomParameters::from(custom_env_vars)),
            ),
            path_cache: OnceCell::from(path),
        })
    }
}

impl ReplaceBasePath for ParametersDir {
    fn replace_base_path(&self, base_path: Arc<Path>) -> Self {
        let Self {
            base_path: _,
            target_name,
            custom_parameters,
            path_cache: _,
        } = self;
        clone!(target_name);
        clone!(custom_parameters);
        Self {
            base_path,
            target_name,
            custom_parameters,
            path_cache: Default::default(),
        }
    }
}

impl ParametersDir {
    pub fn base_path(&self) -> &Arc<Path> {
        &self.base_path
    }
    pub fn target_name(&self) -> &ProperDirname {
        &self.target_name
    }
    pub fn custom_parameters(&self) -> &CheckedOrUncheckedCustomParameters {
        &self.custom_parameters
    }
}

impl TryFrom<Arc<Path>> for KeyDir {
    type Error = anyhow::Error;

    fn try_from(path: Arc<Path>) -> std::result::Result<Self, Self::Error> {
        let (commit_id, parent_dir) = parse_path_filename(&path)?;
        let parent = ParametersDir::try_from(parent_dir.into_arc_path())?.into();
        Ok(Self {
            parent,
            commit_id,
            path_cache: OnceCell::from(path),
        })
    }
}

impl ReplaceBasePath for KeyDir {
    fn replace_base_path(&self, base_path: Arc<Path>) -> Self {
        let Self {
            parent,
            commit_id,
            path_cache: _,
        } = self;
        let parent = parent.replace_base_path(base_path).into();
        clone!(commit_id);
        Self {
            parent,
            commit_id,
            path_cache: Default::default(),
        }
    }
}

impl ToPath for KeyDir {
    fn to_path(&self) -> &Arc<Path> {
        let Self {
            parent,
            commit_id,
            path_cache,
        } = self;
        if path_cache.get().is_none() {
            let path = parent.to_path().append(commit_id.to_string());
            _ = path_cache.set(path.into());
        }
        path_cache.get().unwrap()
    }
}

impl SubDirs for KeyDir {
    type Target = RunDir;

    fn append_str(self: Arc<Self>, file_name: &str) -> Result<Self::Target> {
        Ok(self.append(file_name.parse()?))
    }
}

impl KeyDir {
    pub fn from_base_target_params(
        output_base_dir: Arc<Path>,
        target_name: ProperDirname,
        RunParameters {
            commit_id,
            custom_parameters,
        }: &RunParameters,
    ) -> Arc<Self> {
        let parent = Arc::new(ParametersDir {
            target_name,
            custom_parameters: CheckedOrUncheckedCustomParameters::CustomParameters(
                custom_parameters.clone_arc(),
            ),
            base_path: output_base_dir,
            path_cache: Default::default(),
        });
        let commit_id = commit_id.clone();
        Arc::new(KeyDir {
            commit_id,
            parent,
            path_cache: Default::default(),
        })
    }

    pub fn append(self: Arc<Self>, dir_name: DateTimeWithOffset) -> RunDir {
        RunDir {
            parent: self,
            timestamp: dir_name,
            path_cache: Default::default(),
        }
    }

    pub fn parent(&self) -> &Arc<ParametersDir> {
        &self.parent
    }
    pub fn commit_id(&self) -> &GitHash {
        &self.commit_id
    }
}

impl TryFrom<Arc<Path>> for RunDir {
    type Error = anyhow::Error;

    fn try_from(path: Arc<Path>) -> std::result::Result<Self, Self::Error> {
        let (timestamp, parent_path) = parse_path_filename(&path)?;
        let parent = KeyDir::try_from(parent_path.into_arc_path())?.into();
        Ok(Self {
            parent,
            timestamp,
            path_cache: OnceCell::from(path),
        })
    }
}

impl ReplaceBasePath for RunDir {
    fn replace_base_path(&self, base_path: Arc<Path>) -> Self {
        let Self {
            parent,
            timestamp,
            path_cache: _,
        } = self;
        let parent = parent.replace_base_path(base_path).into();
        clone!(timestamp);
        Self {
            parent,
            timestamp,
            path_cache: Default::default(),
        }
    }
}

impl ToPath for RunDir {
    fn to_path(&self) -> &Arc<Path> {
        let Self {
            parent,
            timestamp,
            path_cache,
        } = self;
        if path_cache.get().is_none() {
            let path = parent.to_path().append(timestamp.to_string());
            _ = path_cache.set(path.into());
        }
        path_cache.get().unwrap()
    }
}

impl RunDir {
    pub fn parent(&self) -> &Arc<KeyDir> {
        &self.parent
    }
    pub fn timestamp(&self) -> &DateTimeWithOffset {
        &self.timestamp
    }

    /// The path to the compressed evobench.log file
    pub fn evobench_log_path(&self) -> PathBuf {
        self.to_path().append("evobench.log.zstd")
    }

    /// The optional output location that target projects can use,
    /// passed to it via the `BENCH_OUTPUT_LOG` env variable then
    /// compressed/moved to this location.
    pub fn bench_output_log_path(&self) -> PathBuf {
        self.to_path().append("bench_output.log.zstd")
    }

    /// The path to the compressed stdout/stderr output from the
    /// target application while running this benchmark.
    pub fn standard_log_path(&self) -> PathBuf {
        self.to_path().append("standard.log.zstd")
    }

    /// Files below a RunDir are normal files (no special type, at
    /// least for now)
    pub fn append(&self, file_name: &ProperFilename) -> PathBuf {
        self.to_path().append(file_name.as_str())
    }

    /// Same as `append` but returns an error if file_name cannot be a
    /// `ProperFilename`.
    pub fn append_str(&self, file_name: &str) -> Result<PathBuf> {
        let proper = ProperFilename::from_str(file_name)
            .map_err(|msg| anyhow!("not a proper file name ({msg}): {file_name:?}"))?;
        Ok(self.append(&proper))
    }
}
