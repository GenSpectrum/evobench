//! Clean up files via `Drop` action

//! Unlike what the `tempfile` offers, can clean paths that we didn't
//! open ourselves, and specify any path.

//! Relying on `Drop` means that kill by e.g. signals without handlers
//! (ctl-c) will prevent the cleanup. Use `PathWithCleanup` instead if
//! that is relevant!

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use crate::info;

pub struct TemporaryFile {
    path: PathBuf,
    // XX currently never used
    file: Option<File>,
}

impl TemporaryFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self) -> Option<&File> {
        self.file.as_ref()
    }
}

impl From<PathBuf> for TemporaryFile {
    fn from(path: PathBuf) -> Self {
        Self { path, file: None }
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => info!("deleted temporary file {:?}", self.path),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => (),
                _ => info!("error deleting temporary file {:?}: {e:#}", self.path),
            },
        }
    }
}
