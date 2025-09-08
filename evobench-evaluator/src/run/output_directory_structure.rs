//! The directory structure for output files.

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, Result};
use run_git::path_util::AppendToPath;

use crate::{
    ctx,
    git::GitHash,
    key::RunParameters,
    serde::{date_and_time::DateTimeWithOffset, proper_dirname::ProperDirname},
    utillib::type_name_short::type_name_short,
};

/// Skips non-directory entries, but requires all directory entries to
/// be convertible to `T`.
fn typed_dir_listing_of_dirs<T: TryFrom<PathBuf, Error = anyhow::Error>>(
    dir_path: &Path,
) -> Result<Vec<T>> {
    std::fs::read_dir(&dir_path)
        .map_err(ctx!("opening dir {dir_path:?}"))?
        .map(|entry| -> Result<Option<T>> {
            let entry: std::fs::DirEntry = entry?;
            let ft = entry.file_type()?;
            if ft.is_dir() {
                Ok(Some(T::try_from(entry.path())?))
            } else {
                Ok(None)
            }
        })
        .filter_map(|r| r.transpose())
        .collect::<Result<_, _>>()
        .map_err(ctx!(
            "getting {} listing for dir {dir_path:?}",
            type_name_short::<T>()
        ))
}

fn parse_path<T: FromStr>(path: &Path) -> Result<T>
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
            Ok(v) => Ok(v),
            Err(e) => {
                bail!(
                    "dir name {file_name_str:?} in {dir:?} \
                     does not parse as {}: {e}",
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

// --- The types ----------------------------------------------------------------

/// The dir representing all of a key except for the commit id
/// (i.e. custom parameters and target name).
#[derive(Debug, Clone)]
pub struct CustomParametersDir(PathBuf);

/// Dir representing all of the key, including commit id at the
/// end. I.e. one level below a `RunParametersDir`.
#[derive(Debug, Clone)]
pub struct KeyDir(PathBuf);

/// Dir with the results for an individual benchmarking run. I.e. one
/// level below a `KeyDir`.
#[derive(Debug, Clone)]
pub struct RunDir(PathBuf);

// --- Their implementations ----------------------------------------------------

impl CustomParametersDir {
    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn key_dirs(&self) -> Result<Vec<KeyDir>> {
        typed_dir_listing_of_dirs(self.path())
    }
}

impl TryFrom<PathBuf> for KeyDir {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> std::result::Result<Self, Self::Error> {
        _ = parse_path::<GitHash>(&path)?;
        Ok(Self(path))
    }
}

impl KeyDir {
    pub fn from_base_target_params(
        output_base_dir: &Path,
        target_name: &ProperDirname,
        run_parameters: &RunParameters,
    ) -> Self {
        KeyDir::try_from(run_parameters.extend_path(output_base_dir.append(target_name.as_str())))
            .expect("self-created paths follow the spec")
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn append(&self, dir_name: &str) -> Result<RunDir> {
        RunDir::try_from(self.path().append(dir_name))
    }

    pub fn run_dirs(&self) -> Result<Vec<RunDir>> {
        typed_dir_listing_of_dirs(self.path())
    }
}

impl TryFrom<PathBuf> for RunDir {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> std::result::Result<Self, Self::Error> {
        _ = parse_path::<DateTimeWithOffset>(&path)?;
        Ok(Self(path))
    }
}

impl RunDir {
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The standard path to the compressed evobench.log file
    pub fn evobench_log_path(&self) -> PathBuf {
        self.path().append("evobench.log.zstd")
    }

    /// Files below a RunDir are normal files (no special type, at
    /// least for now)
    pub fn append(&self, file_name: impl AsRef<Path>) -> PathBuf {
        self.path().append(file_name)
    }
}
