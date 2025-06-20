use std::{
    ffi::OsString,
    fs::File,
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{anyhow, bail, Context, Result};
use ruzstd::StreamingDecoder;

const USING_EXTERNAL_TOOL: bool = false;

#[derive(Debug, PartialEq)]
pub enum Extension<'s> {
    ZStd,
    Other(&'s str),
}

pub fn file_extension<'s, P: AsRef<Path>>(
    path: P,
    expected_suffix: &'s str,
) -> Result<Extension<'s>> {
    let path = path.as_ref();
    let ext = path.extension().ok_or_else(|| {
        anyhow!("missing file extension, expecting {expected_suffix:?} or \"zstd\": {path:?}")
    })?;

    match ext.to_string_lossy().as_ref() {
        "zstd" => {
            let stem = path.with_extension("");
            let ext2 = stem.extension().ok_or_else(|| {
                anyhow!(
                    "missing second file extension, after \"zstd\", \
                     expecting {expected_suffix:?}: {path:?}"
                )
            })?;
            match ext2.to_string_lossy().as_ref() {
                s if &*s == expected_suffix => Ok(Extension::ZStd),
                _ => bail!(
                    "unknown second file extension {ext2:?} after \"zstd\", \
                     expecting {expected_suffix:?}: {path:?}"
                ),
            }
        }
        s if &*s == expected_suffix => Ok(Extension::Other(expected_suffix)),
        _ => bail!(
            "unknown file extension {ext:?}, expecting {expected_suffix:?} \
             or \"zstd\": {path:?}"
        ),
    }
}

#[test]
fn t_file_extension() {
    use Extension::*;
    let ok = |a: &str, b: &'static str| {
        file_extension(a, b).expect("test call should not give an error")
    };
    let err = |a: &str, b: &'static str| {
        file_extension(a, b)
            .err()
            .expect("test call should give an error")
            .to_string()
    };
    assert_eq!(ok("foo.x", "x"), Other("x"));
    assert_eq!(ok("foo.x.zstd", "x"), ZStd);
    assert_eq!(ok("foo.z.x", "x"), Other("x"));
    assert_eq!(ok("foo.z.x.zstd", "x"), ZStd);
    assert_eq!(
        err("foo.x", "y"),
        "unknown file extension \"x\", expecting \"y\" or \"zstd\": \"foo.x\""
    );
    assert_eq!(
        err("foo.x.zstd", "y"),
        "unknown second file extension \"x\" after \"zstd\", expecting \"y\": \"foo.x.zstd\""
    );
    assert_eq!(
        err("foo.zstd", "y"),
        "missing second file extension, after \"zstd\", expecting \"y\": \"foo.zstd\""
    );
    assert_eq!(
        err("foo", "y"),
        "missing file extension, expecting \"y\" or \"zstd\": \"foo\""
    );
}

/// Transparently decompress zstd files if they have a .zstd suffix;
/// after that, expecting the `expected_suffix` (which must be given
/// *without* a leading dot)
pub fn decompressed_file(path: &Path, expected_suffix: &str) -> Result<Box<dyn Read>> {
    let ext = file_extension(path, expected_suffix)?;

    let file_open = || File::open(path).with_context(|| anyhow!("opening file {path:?}"));

    match ext {
        Extension::ZStd => {
            if USING_EXTERNAL_TOOL {
                let mut c = Command::new("zstd");
                let args: Vec<OsString> = vec!["-dcf".into(), "--".into(), path.into()];
                c.args(args);
                c.stdout(Stdio::piped());
                let child = c
                    .spawn()
                    .with_context(|| anyhow!("opening file {path:?}"))?;
                Ok(Box::new(child.stdout.expect("present since configured")))
            } else {
                let input = file_open()?;
                Ok(Box::new(
                    StreamingDecoder::new(input)
                        .with_context(|| anyhow!("zstd-decoding {path:?}"))?,
                ))
            }
        }
        Extension::Other(_) => Ok(Box::new(file_open()?)),
    }
}
