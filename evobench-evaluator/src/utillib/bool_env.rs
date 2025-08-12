use anyhow::{bail, Result};

/// Get the boolean value from env variable `name`. If missing, if
/// `default` is given, return that, otherwise return an
/// error. Returns errors for invalid values (anything else than
/// `0|1|f|t|n|y`).
pub fn get_env_bool(name: &str, default: Option<bool>) -> Result<bool> {
    match std::env::var("DEBUG_LINEAR") {
        Ok(s) => match s.as_str() {
            "1" | "t" | "y" => Ok(true),
            "0" | "f" | "n" => Ok(false),
            _ => bail!("invalid value for env variable {name:?}"),
        },
        Err(e) => match e {
            std::env::VarError::NotPresent => {
                if let Some(default) = default {
                    Ok(default)
                } else {
                    bail!("env variable {name:?} is missing")
                }
            }
            std::env::VarError::NotUnicode(_) => bail!("non-utf8 string in env var DEBUG_LINEAR"),
        },
    }
}
