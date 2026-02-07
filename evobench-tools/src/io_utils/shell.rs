use anyhow::{Result, bail};

/// Returns what `$SHELL` is set to, or otherwise `bash`
pub fn preferred_shell() -> Result<String> {
    match std::env::var("SHELL") {
        Ok(s) => Ok(s),
        Err(e) => match e {
            std::env::VarError::NotPresent => Ok("bash".into()),
            std::env::VarError::NotUnicode(os_string) => {
                bail!("the SHELL environment variable is not in unicode: {os_string:?}")
            }
        },
    }
}
