use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use super::home::home_dir;

/// Change paths starting with `~/` to replace the `~` with the user's
/// home directory. Careful: if path is not representable as unicode
/// string, no expansion is attempted!
pub fn path_resolve_home(path: &Path) -> Result<PathBuf> {
    if let Some(path_str) = path.to_str() {
        if path_str.starts_with("~") {
            let home = home_dir()?;
            if path_str == "~" {
                return Ok(home.to_owned());
            }
            if path_str.starts_with("~/") {
                let home_str = home.to_str().ok_or_else(|| {
                    anyhow!("home dir {home:?} can't be represented as unicode string")
                })?;
                return Ok(format!("{home_str}/{}", &path_str[2..]).into());
            }
        }
    }
    Ok(path.to_owned())
}
