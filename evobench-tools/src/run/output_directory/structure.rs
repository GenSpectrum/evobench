//! The directory structure for output files.

use std::{
    collections::BTreeMap,
    fmt::Display,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, OnceLock},
};

use anyhow::{Result, anyhow, bail};
use cj_path_util::path_util::AppendToPath;
use derive_more::From;
use kstring::KString;

use crate::{
    clone, ctx,
    git::GitHash,
    run::{
        config::JobTemplate,
        env_vars::AllowableCustomEnvVar,
        key::{CustomParameters, ExtendPath, RunParameters, UncheckedCustomParameters},
    },
    serde_types::{
        allowed_env_var::AllowedEnvVar, date_and_time::DateTimeWithOffset,
        proper_dirname::ProperDirname, proper_filename::ProperFilename,
    },
    utillib::{
        arc::CloneArc, into_arc_path::IntoArcPath, invert::Invert, path_is_top::PathIsTop,
        type_name_short::type_name_short,
    },
};

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
    path_cache: OnceLock<Arc<Path>>,
}

/// Dir representing all of the key, including commit id at the
/// end. I.e. one level below a `RunParametersDir`.
#[derive(Debug, Clone)]
pub struct KeyDir {
    parent: Arc<ParametersDir>,
    commit_id: GitHash,
    path_cache: OnceLock<Arc<Path>>,
}

/// Dir with the results for an individual benchmarking run. I.e. one
/// level below a `KeyDir`.
#[derive(Debug, Clone)]
pub struct RunDir {
    parent: Arc<KeyDir>,
    timestamp: DateTimeWithOffset,
    path_cache: OnceLock<Arc<Path>>,
}

/// Any kind of *Dir
#[derive(derive_more::From)]
pub enum OutputSubdir {
    ParametersDir(Arc<ParametersDir>),
    KeyDir(Arc<KeyDir>),
    RunDir(Arc<RunDir>),
}

// --- Their implementations ----------------------------------------------------

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
    fn sub_dirs(self: &Arc<Self>) -> Result<impl Iterator<Item = Result<Self::Target>>> {
        let dir_path = self.to_path().to_owned();
        Ok(std::fs::read_dir(&dir_path)
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
            .filter_map({
                move |r| {
                    r.map_err(ctx!(
                        "getting {} listing for dir {dir_path:?}",
                        type_name_short::<Self>()
                    ))
                    .transpose()
                }
            }))
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
                    "dir name {file_name_str:?} in {path:?} \
                     does not parse as {}: {e:#}",
                    type_name_short::<T>()
                )
            }
        }
    } else {
        let lossy1 = file_name.to_string_lossy();
        let lossy: &str = lossy1.as_ref();
        bail!("can't decode dir name to string: {lossy:?} in {path:?}");
    }
}

/// Need to be able to handle unchecked custom parameters for parsing
/// since there's no config file for parsing a file system, or more to
/// the point, when the config changes, the file system must still be
/// readable, thus the representation must remain independent of the
/// config. But allow to represent both. Do this at runtime for
/// ~simplicity. (Maybe this should be moved to where the contained
/// types are defined, which maybe shouldn't all be in `key.rs`.)
#[derive(Debug, Clone, From)]
pub enum CheckedOrUncheckedCustomParameters {
    UncheckedCustomParameters(#[from] Arc<UncheckedCustomParameters>),
    CustomParameters(#[from] Arc<CustomParameters>),
}

impl CheckedOrUncheckedCustomParameters {
    pub fn extend_path(&self, path: PathBuf) -> PathBuf {
        match self {
            CheckedOrUncheckedCustomParameters::UncheckedCustomParameters(v) => v.extend_path(path),
            CheckedOrUncheckedCustomParameters::CustomParameters(v) => v.extend_path(path),
        }
    }

    pub fn get(&self, key: &AllowedEnvVar<AllowableCustomEnvVar>) -> Option<&str> {
        match self {
            CheckedOrUncheckedCustomParameters::UncheckedCustomParameters(v) => {
                v.btree_map().get(key).map(AsRef::as_ref)
            }
            CheckedOrUncheckedCustomParameters::CustomParameters(v) => {
                v.btree_map().get(key).map(AsRef::as_ref)
            }
        }
    }
}

impl Display for CheckedOrUncheckedCustomParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckedOrUncheckedCustomParameters::UncheckedCustomParameters(v) => v.fmt(f),
            CheckedOrUncheckedCustomParameters::CustomParameters(v) => v.fmt(f),
        }
    }
}

impl PartialEq for ParametersDir {
    fn eq(&self, other: &Self) -> bool {
        self.to_path().eq(other.to_path())
    }
}
impl Eq for ParametersDir {}

impl PartialOrd for ParametersDir {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.to_path().partial_cmp(other.to_path())
    }
}

impl Ord for ParametersDir {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_path().cmp(other.to_path())
    }
}

impl ToPath for ParametersDir {
    fn to_path(&self) -> &Arc<Path> {
        let Self {
            base_path,
            target_name,
            custom_parameters,
            path_cache,
        } = self;
        path_cache.get_or_init(|| {
            custom_parameters
                .extend_path(base_path.append(target_name.as_str()))
                .into()
        })
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
                            if parent.is_top() {
                                bail!(
                                    "parsing {} {:?}: missing target segment left of the var segments",
                                    type_name_short::<Self>(),
                                    path
                                );
                            }
                            current_path = parent;
                        } else {
                            unreachable!("because file_name() above already failed, right?")
                        }
                    } else {
                        target_name = ProperDirname::from_str(dir_name_str).map_err(|msg| {
                            anyhow!("not a proper directory name: {dir_name_str:?}: {msg}")
                        })?;
                        custom_env_vars = current_vars;
                        if let Some(parent) = current_path.parent() {
                            base_path = parent.into();
                        } else {
                            // This never happens, right?
                            bail!("path is missing a base_dir part (1): {path:?}")
                        }
                        break;
                    }
                } else {
                    if current_path.is_top() {
                        bail!("path is missing a target or base_dir part: {path:?}")
                    }
                    bail!("path {path:?} contains a '..' or '.' part: {current_path:?}")
                }
            }
        }
        Ok(Self {
            base_path,
            target_name,
            custom_parameters: CheckedOrUncheckedCustomParameters::UncheckedCustomParameters(
                Arc::new(UncheckedCustomParameters::from(custom_env_vars)),
            ),
            path_cache: path.into(),
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

// (There's not impl JobTemplate yet, also JobTemplate is not in its
// own dir but config.rs, so, just put this here.)
impl JobTemplate {
    pub fn to_parameters_dir(&self, base_path: Arc<Path>) -> ParametersDir {
        ParametersDir::from_job_template(base_path, self)
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

    pub fn from_job_template(base_path: Arc<Path>, job_template: &JobTemplate) -> Self {
        let JobTemplate {
            priority: _,
            initial_boost: _,
            command,
            custom_parameters,
        } = job_template;
        let target_name = command.target_name.clone();
        let custom_parameters = custom_parameters.clone_arc().into();
        Self {
            base_path,
            target_name,
            custom_parameters,
            path_cache: Default::default(),
        }
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
            path_cache: path.into(),
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
        path_cache.get_or_init(|| parent.to_path().append(commit_id.to_string()).into())
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
            path_cache: path.into(),
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
        path_cache.get_or_init(|| parent.to_path().append(timestamp.to_string()).into())
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

// Implement conversions from the bare (non-Arc) variants
macro_rules! def_output_subdir_from {
    { $t:tt } => {
        impl From<$t> for OutputSubdir {
            fn from(value: $t) -> Self {
                Self::$t(Arc::new(value))
            }
        }
    }
}
def_output_subdir_from!(ParametersDir);
def_output_subdir_from!(KeyDir);
def_output_subdir_from!(RunDir);

impl OutputSubdir {
    pub fn replace_base_path(self, path: Arc<Path>) -> Self {
        match self {
            OutputSubdir::ParametersDir(v) => v.replace_base_path(path).into(),
            OutputSubdir::KeyDir(v) => v.replace_base_path(path).into(),
            OutputSubdir::RunDir(v) => v.replace_base_path(path).into(),
        }
    }

    pub fn to_path(&self) -> &Arc<Path> {
        match self {
            OutputSubdir::ParametersDir(v) => v.to_path(),
            OutputSubdir::KeyDir(v) => v.to_path(),
            OutputSubdir::RunDir(v) => v.to_path(),
        }
    }
}

/// Attempt to parse as all levels, with the deepest type first.
impl TryFrom<Arc<Path>> for OutputSubdir {
    type Error = anyhow::Error;

    fn try_from(path: Arc<Path>) -> std::result::Result<Self, Self::Error> {
        // By exchanging the Ok and Err cases via .invert(), `?` ends
        // with the first successful result (converted into
        // OutputSubdir). The code flow stays in the closure while
        // there are errors. Invert the meaning back outside.
        (|| -> Result<anyhow::Error, OutputSubdir> {
            let e1 = RunDir::try_from(path.clone_arc()).invert()?;
            let e2 = KeyDir::try_from(path.clone_arc()).invert()?;
            let e3 = ParametersDir::try_from(path.clone_arc()).invert()?;
            Ok(anyhow!(
                "can't parse path {path:?}\n\
                 - as RunDir: {e1:#}\n\
                 - as KeyDir: {e2:#}\n\
                 - as ParametersDir: {e3:#}"
            ))
        })()
        .invert()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_parameters_dir() {
        let path = "/home/evobench/silo-benchmark-outputs/api/CONCURRENCY=120/DATASET=SC2open\
                    /RANDOMIZED=1/REPEAT=1/SORTED=0"
            .into_arc_path();
        let d = ParametersDir::try_from(path.clone_arc()).unwrap();
        assert_eq!(
            d.base_path(),
            &"/home/evobench/silo-benchmark-outputs".into_arc_path()
        );
        assert_eq!(d.target_name().as_str(), "api");

        let p = |name: &str| -> AllowedEnvVar<AllowableCustomEnvVar> { name.parse().unwrap() };
        assert_eq!(d.custom_parameters().get(&p("CONCURRENCY")), Some("120"));
        assert_eq!(d.custom_parameters().get(&p("DATASET")), Some("SC2open"));
        assert_eq!(d.custom_parameters().get(&p("SORTED")), Some("0"));

        assert_eq!(d.to_path(), &path);

        let new_base_path = "foo".into_arc_path();
        let new_path = "foo/api/CONCURRENCY=120/DATASET=SC2open/RANDOMIZED=1/REPEAT=1/SORTED=0"
            .into_arc_path();
        let d2 = d.replace_base_path(new_base_path.clone_arc());
        assert_eq!(d2.base_path(), &new_base_path);
        assert_eq!(d2.to_path(), &new_path);
    }

    #[test]
    fn t_output_subdir() -> Result<(), String> {
        let t1 = |s: &str| -> Result<OutputSubdir> {
            let dir = OutputSubdir::try_from(s.into_arc_path())?;
            Ok(dir.replace_base_path("BASE".into_arc_path()))
        };
        let t2 = |s: &str| -> Result<PathBuf> {
            let dir = t1(s)?;
            Ok(dir.to_path().to_path_buf())
        };
        let t = |s: &str| -> Result<String, String> {
            match t2(s) {
                Ok(p) => Ok(p.to_string_lossy().to_string()),
                Err(e) => Err(e.to_string()),
            }
        };

        assert_eq!(&t("/foo=1//bar=2/fa")?, "BASE/fa");
        assert_eq!(&t("api/foo=1//bar=2/")?, "BASE/api/bar=2/foo=1");
        assert_eq!(&t("um/api/foo=1/bar=2/")?, "BASE/api/bar=2/foo=1");
        assert_eq!(&t("/api/foo=1/bar=2/")?, "BASE/api/bar=2/foo=1");
        // parent() skips over `.`
        assert_eq!(&t("um/api/foo=1/./bar=2/")?, "BASE/api/bar=2/foo=1");
        // `..`.file_name() returns None
        assert_eq!(
            &t("um/api/foo=1/baz=3/../bar=2/").err().unwrap(),
            "can't parse path \"um/api/foo=1/baz=3/../bar=2/\"\n- as RunDir: dir name \"bar=2\" in \"um/api/foo=1/baz=3/../bar=2/\" does not parse as DateTimeWithOffset: input contains invalid characters\n- as KeyDir: dir name \"bar=2\" in \"um/api/foo=1/baz=3/../bar=2/\" does not parse as GitHash: not a git hash of 40 hex bytes: \"bar=2\"\n- as ParametersDir: path \"um/api/foo=1/baz=3/../bar=2/\" contains a '..' or '.' part: \"um/api/foo=1/baz=3/..\""
        );
        assert_eq!(
            &t("api/foo=1/bar=2/09193b52688a964956b3fae0f52eeae471adc027")?,
            "BASE/api/bar=2/foo=1/09193b52688a964956b3fae0f52eeae471adc027"
        );
        assert_eq!(
            &t("api/foo=1/bar=2/09193b52688a964956b3fae0f52eeae471adc027/\
                2026-02-02T11:26:48.563793486+00:00")?,
            "BASE/api/bar=2/foo=1/09193b52688a964956b3fae0f52eeae471adc027/\
             2026-02-02T11:26:48.563793486+00:00"
        );
        // Parses as the commit id *is* a ProperDirname thus used as
        // target name! Hmm, could it check for the kinds of errors in
        // earlier parses and decide on that? If it is successful
        // parsing 2 out of 3 then... basically do those counts. 2
        // successful parses > 1 parse. Although should perhaps
        // require "non-trivial", too.
        assert_eq!(
            &t("/foo=1//bar=2/09193b52688a964956b3fae0f52eeae471adc027")?,
            "BASE/09193b52688a964956b3fae0f52eeae471adc027"
        );
        // Interesting case since the multi-error message feature is
        // actually useful here:
        assert_eq!(
            &t("/foo=1//bar=2/09193b52688a964956b3fae0f52eeae471adc027/\
                2026-02-02T11:26:48.563793486+00:00")
            .err()
            .unwrap(),
            "can't parse path \"/foo=1//bar=2/09193b52688a964956b3fae0f52eeae471adc027/2026-02-02T11:26:48.563793486+00:00\"\n- as RunDir: parsing ParametersDir \"/foo=1//bar=2\": missing target segment left of the var segments\n- as KeyDir: dir name \"2026-02-02T11:26:48.563793486+00:00\" in \"/foo=1//bar=2/09193b52688a964956b3fae0f52eeae471adc027/2026-02-02T11:26:48.563793486+00:00\" does not parse as GitHash: not a git hash of 40 hex bytes: \"2026-02-02T11:26:48.563793486+00:00\"\n- as ParametersDir: not a proper directory name: \"2026-02-02T11:26:48.563793486+00:00\": a file name (not path), must not contain '/', '\\n', '\\0', and must not be \".\", \"..\", the empty string, or longer than 255 bytes, and not have a file extension"
        );
        // `.` is not dropped: it's only skiped by
        // parent(). file_name() then yields another Null.
        assert_eq!(
            &t(".").err().unwrap(),
            "can't parse path \".\"\n- as RunDir: path is missing a file name\n- as KeyDir: path is missing a file name\n- as ParametersDir: path \".\" contains a '..' or '.' part: \".\""
        );
        assert_eq!(
            &t("./foo=1").err().unwrap(),
            "can't parse path \"./foo=1\"\n- as RunDir: dir name \"foo=1\" in \"./foo=1\" does not parse as DateTimeWithOffset: input contains invalid characters\n- as KeyDir: dir name \"foo=1\" in \"./foo=1\" does not parse as GitHash: not a git hash of 40 hex bytes: \"foo=1\"\n- as ParametersDir: path \"./foo=1\" contains a '..' or '.' part: \".\""
        );
        assert_eq!(&t("./a/foo=1")?, "BASE/a/foo=1");
        assert_eq!(&t("./a/./foo=1")?, "BASE/a/foo=1");
        // Only if the `.` is right of a `..` it happens
        assert_eq!(
            &t("./../.").err().unwrap(),
            "can't parse path \"./../.\"\n- as RunDir: path is missing a file name\n- as KeyDir: path is missing a file name\n- as ParametersDir: path \"./../.\" contains a '..' or '.' part: \"./../.\""
        );
        assert_eq!(&t("a/.././b")?, "BASE/b");
        assert_eq!(&t("a/../b/.")?, "BASE/b");
        Ok(())
    }
}
