//! More composable/controllable utilities for handling temporary files?

use std::{
    fs::File,
    io::Write,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use nix::{
    errno::Errno,
    unistd::{chown, getpid, gettid, Gid, Uid},
};
use run_git::path_util::AppendToPath;

use crate::info;

#[derive(Debug, thiserror::Error)]
pub enum TempfileError {
    #[error("path is missing parent directory part")]
    MissingParent,
    #[error("path is missing file name part")]
    MissingFileName,
    #[error("IO error while {0}: {1:#}")]
    IOError(&'static str, std::io::Error),
    #[error("IO error while {0}: {1:#}")]
    IOErrno(&'static str, Errno),
}

/// Append a suffix `.tmp~..-..` where the numbers are pid and tid
pub fn temp_path(target_path: impl AsRef<Path>) -> Result<PathBuf, TempfileError> {
    let target_path = target_path.as_ref();
    let dir = target_path
        .parent()
        .ok_or_else(|| TempfileError::MissingParent)?;
    let file_name = target_path
        .file_name()
        .ok_or_else(|| TempfileError::MissingFileName)?;
    let mut file_name: Vec<u8> = file_name.to_string_lossy().to_string().into();
    let pid = getpid();
    let tid = gettid();
    write!(&mut file_name, ".tmp~{pid}-{tid}").expect("nofail: no IO");
    let file_name =
        String::from_utf8(file_name).expect("nofail: was a string, with strings appended");
    Ok(dir.append(file_name))
}

// #[test]
// fn t() {
//     assert_eq!("foo", temp_path("fun").expect("no err").to_string_lossy());
// }

#[derive(Debug, Clone)]
pub struct TempfileOpts {
    pub target_path: PathBuf,
    pub retain_tempfile: bool,
    pub migrate_access: bool,
}

impl TempfileOpts {
    pub fn tempfile(self) -> Result<Tempfile, TempfileError> {
        Tempfile::try_from(self)
    }
}

#[derive(Debug)]
pub struct Tempfile {
    pub opts: TempfileOpts,
    pub temp_path: PathBuf,
}

impl TryFrom<TempfileOpts> for Tempfile {
    type Error = TempfileError;

    fn try_from(opts: TempfileOpts) -> Result<Self, TempfileError> {
        let temp_path = temp_path(&opts.target_path)?;
        Ok(Tempfile { opts, temp_path })
    }
}

impl Tempfile {
    pub fn finish(mut self) -> Result<(), TempfileError> {
        self.opts.retain_tempfile = true; // tell Drop that it should do nothing
        let Self {
            opts:
                TempfileOpts {
                    ref target_path,
                    retain_tempfile: _,
                    migrate_access,
                },
            ref temp_path,
        } = self;
        let meta = if migrate_access {
            match target_path.metadata() {
                Ok(m) => Some(m),
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => None,
                    _ => return Err(TempfileError::IOError("getting metadata on target file", e)),
                },
            }
        } else {
            None
        };
        if let Some(meta) = meta {
            // XX iirc when one sets setuid/setgid, user needs to be
            // set first? Also, accessibility race, OK?
            let uid = meta.uid();
            let gid = meta.gid();
            chown(
                temp_path.into(),
                Some(Uid::from_raw(uid)),
                Some(Gid::from_raw(gid)),
            )
            .map_err(|e| TempfileError::IOErrno("copying owner/group to new file", e))?;

            let perms = meta.permissions();
            let mode = perms.mode();
            // Is the way via mode really necessary, or pass perms directly??
            std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(mode))
                .map_err(|e| TempfileError::IOError("copying permissions to new file", e))?;
        }
        std::fs::rename(temp_path, target_path)
            .map_err(|e| TempfileError::IOError("renaming to target", e))?;
        Ok(())
    }
}

impl Drop for Tempfile {
    fn drop(&mut self) {
        let Self {
            opts:
                TempfileOpts {
                    target_path: _,
                    retain_tempfile,
                    migrate_access: _,
                },
            temp_path,
        } = self;
        if !*retain_tempfile {
            match std::fs::remove_file(&*temp_path) {
                Ok(()) => (),
                Err(e) => match e.kind() {
                    std::io::ErrorKind::NotFound => (),
                    _ => info!("error deleting temporary file {:?}: {e:#}", temp_path),
                },
            }
        }
    }
}

// XX todo?
pub struct TempfileWithFlush {
    pub tempfile: Tempfile,
    pub file: File,
}
