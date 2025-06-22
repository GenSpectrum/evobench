use std::{
    fs::{create_dir, rename},
    path::Path,
};

use anyhow::{anyhow, Context, Result};

pub fn xrename(from: &Path, to: &Path) -> Result<()> {
    rename(from, to).with_context(|| anyhow!("renaming {from:?} to {to:?}"))?;
    Ok(())
}

/// Returns true if the directory was created, false if already
/// existed. `ctx` should be something like "queues base directory",
/// made part of the error message.
pub fn create_dir_if_not_exists<P: AsRef<Path>>(path: P, ctx: &str) -> Result<bool> {
    let path = path.as_ref();
    match create_dir(path) {
        Ok(()) => Ok(true),
        Err(e) => match e.kind() {
            std::io::ErrorKind::AlreadyExists => Ok(false),
            _ => Err(e).with_context(|| anyhow!("creating {ctx} {path:?}")),
        },
    }
}
