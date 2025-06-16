use std::path::{Path, PathBuf};

use lazy_static::lazy_static;

#[derive(thiserror::Error, Debug)]
pub enum HomeError {
    #[error(
        "path given in HOME environment variable does not point to \
             an existing directory: {0:?}"
    )]
    NotExist(PathBuf),
    #[error("HOME environment variable is not set")]
    NotSet,
}

fn get_home_dir() -> Result<PathBuf, HomeError> {
    if let Some(var) = std::env::var_os("HOME") {
        let path: PathBuf = var.into();
        if path.is_dir() {
            Ok(path)
        } else {
            Err(HomeError::NotExist(path))
        }
    } else {
        Err(HomeError::NotSet)
    }
}

lazy_static! {
    static ref HOME_DIR: Result<PathBuf, HomeError> = get_home_dir();
}

pub fn home_dir() -> Result<&'static Path, &'static HomeError> {
    match &*HOME_DIR {
        Ok(v) => Ok(v),
        Err(e) => Err(e),
    }
}
