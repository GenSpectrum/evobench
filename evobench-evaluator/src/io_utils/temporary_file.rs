//! Clean up files in `Drop`. Unlike what the `tempfile` offers, can
//! clean paths that we didn't open ourselves, and specify any path.

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use crate::info;

pub struct TemporaryFile {
    path: PathBuf,
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
            Err(e) => info!("error deleting temporary file {:?}: {e:#}", self.path),
        }
    }
}
