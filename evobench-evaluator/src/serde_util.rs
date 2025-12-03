use std::path::Path;

use anyhow::Result;
use serde::de::DeserializeOwned;

use crate::ctx;

/// Read the file at `path` to RAM, then deserialize it using
/// `serde_json`. If you want to support json5 or ron, use
/// `config_file.rs` instead.
pub fn serde_read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    (|| -> Result<_> {
        let s = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&s)?)
    })()
    .map_err(ctx!("reading file {path:?}"))
}
