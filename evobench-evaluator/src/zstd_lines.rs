use std::{
    ffi::OsString,
    fs::File,
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{anyhow, bail, Context, Result};

/// Transparently decompress zstd files if they have a .zstd suffix;
/// after that, expecting the `expected_suffix` (XX well, currently
/// not checking the sub-suffix if it has .zstd suffix).
pub fn decompressed_file(path: &Path, expected_suffix: &str) -> Result<Box<dyn Read>> {
    let ext = path.extension().ok_or_else(|| {
        anyhow!("missing file extension, expecting {expected_suffix:?} or \".zstd\": {path:?}")
    })?;

    let is_compressed = match ext.to_string_lossy().as_ref() {
        "zstd" => true,
        s if &*s == expected_suffix => false,
        _ => bail!("unknown file extension {ext:?}, expecting .log or .zstd: {path:?}"),
    };

    if is_compressed {
        let mut c = Command::new("zstd");
        let args: Vec<OsString> = vec!["-dcf".into(), "--".into(), path.into()];
        c.args(args);
        c.stdout(Stdio::piped());
        let child = c
            .spawn()
            .with_context(|| anyhow!("opening file {path:?}"))?;
        let out = child.stdout.expect("present since configured");
        Ok(Box::new(out))
    } else {
        Ok(Box::new(
            File::open(path).with_context(|| anyhow!("opening file {path:?}"))?,
        ))
    }
}
