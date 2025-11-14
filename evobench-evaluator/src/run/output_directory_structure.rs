//! The directory structure for output files.

use std::{
    collections::BTreeMap,
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{anyhow, bail, Result};
use kstring::KString;
use run_git::path_util::AppendToPath;

use crate::{
    ctx,
    git::GitHash,
    key::RunParameters,
    run::env_vars::AllowableCustomEnvVar,
    serde::{
        allowed_env_var::AllowedEnvVar, date_and_time::DateTimeWithOffset,
        proper_dirname::ProperDirname, proper_filename::ProperFilename,
    },
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

    /// Returns `(target_name, custom_env_vars)`. This returns custom
    /// env vars that are *not* checked against the config; they are
    /// the raw values, but they still do follow the requirements for
    /// env var names. Similarly, `target_name` is not checked against
    /// anything (other than being a directory name).
    pub fn parse(
        &self,
    ) -> Result<(
        ProperDirname,
        BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, KString>,
    )> {
        let mut path = self.path();
        let mut params = BTreeMap::new();
        loop {
            if let Some(dir_name) = path.file_name() {
                let dir_name_str = dir_name.to_str().ok_or_else(|| {
                    anyhow!(
                        "directory segment can't be decoded as string: {:?} in {:?}",
                        dir_name.to_string_lossy().as_ref(),
                        self.path()
                    )
                })?;
                if let Some((var_name, val)) = dir_name_str.split_once('=') {
                    let key = AllowedEnvVar::from_str(var_name)?;
                    let val = KString::from_ref(val);
                    params.insert(key, val);

                    if let Some(parent) = path.parent() {
                        path = parent;
                    } else {
                        bail!(
                            "parsing {} {:?}: ran out of parent segments",
                            type_name_short::<Self>(),
                            self.path()
                        );
                    }
                } else {
                    let target_name = ProperDirname::from_str(dir_name_str).map_err(|msg| {
                        anyhow!("not a proper directory name: {dir_name_str:?}: {msg}")
                    })?;
                    return Ok((target_name, params));
                }
            }
        }
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

    pub fn parse(
        &self,
    ) -> Result<(
        ProperDirname,
        BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, KString>,
        GitHash,
    )> {
        let dir_name = self
            .path()
            .file_name()
            .expect("filename guaranteed by construction");
        let dir_name_str = dir_name
            .to_str()
            .expect("guaranteed parseable by construction");
        let commit = GitHash::from_str(dir_name_str)?;
        let parent_path = self
            .path()
            .parent()
            .expect("parent guaranteed by construction");
        let params_dir = CustomParametersDir(parent_path.to_owned());
        let (target_name, params) = params_dir.parse()?;
        Ok((target_name, params, commit))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn append(&self, dir_name: &DateTimeWithOffset) -> Result<RunDir> {
        RunDir::try_from(self.path().append(dir_name.as_str()))
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
    pub fn parse(
        &self,
    ) -> Result<(
        ProperDirname,
        BTreeMap<AllowedEnvVar<AllowableCustomEnvVar>, KString>,
        GitHash,
        DateTimeWithOffset,
    )> {
        let dir_name = self
            .path()
            .file_name()
            .expect("filename guaranteed by construction");
        let dir_name_str = dir_name
            .to_str()
            .expect("guaranteed parseable by construction");
        let timestamp = DateTimeWithOffset::from_str(dir_name_str)?;
        let parent_path = self
            .path()
            .parent()
            .expect("parent guaranteed by construction");
        let key_dir = KeyDir(parent_path.to_owned());
        let (target_name, params, commit) = key_dir.parse()?;
        Ok((target_name, params, commit, timestamp))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The path to the compressed evobench.log file
    pub fn evobench_log_path(&self) -> PathBuf {
        self.path().append("evobench.log.zstd")
    }

    /// The optional output location that target projects can use,
    /// passed to it via the `BENCH_OUTPUT_LOG` env variable then
    /// compressed/moved to this location.
    pub fn bench_output_log_path(&self) -> PathBuf {
        self.path().append("bench_output.log.zstd")
    }

    /// The path to the compressed stdout/stderr output from the
    /// target application while running this benchmark.
    pub fn standard_log_path(&self) -> PathBuf {
        self.path().append("standard.log.zstd")
    }

    pub fn target_name(&self) -> Result<ProperDirname> {
        Ok(self.parse()?.0)
    }

    /// Files below a RunDir are normal files (no special type, at
    /// least for now)
    pub fn append(&self, file_name: &ProperFilename) -> PathBuf {
        self.path().append(file_name.as_str())
    }

    /// Same as `append` but returns an error if file_name cannot be a
    /// `ProperFilename`.
    pub fn append_str(&self, file_name: &str) -> Result<PathBuf> {
        let proper = ProperFilename::from_str(file_name)
            .map_err(|msg| anyhow!("not a proper file name ({msg}): {file_name:?}"))?;
        Ok(self.append(&proper))
    }
}
