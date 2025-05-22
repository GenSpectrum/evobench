use std::{fs::rename, path::Path};

use anyhow::{anyhow, Context, Result};

pub fn xrename(from: &Path, to: &Path) -> Result<()> {
    rename(from, to).with_context(|| anyhow!("renaming {from:?} to {to:?}"))?;
    Ok(())
}
