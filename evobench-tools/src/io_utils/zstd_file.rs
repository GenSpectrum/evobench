use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Read,
    path::Path,
    process::{ChildStdout, Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};
use ruzstd::{FrameDecoder, StreamingDecoder};

use crate::ctx;

// For decompression; compression is always done via tool.
const USING_EXTERNAL_TOOL: bool = false;

#[derive(Debug, PartialEq)]
enum Extension {
    ZStd,
    Other,
}

// Expect a file extension in `path`, return whether it is "zstd" or
// the `expected_suffix`. Anything else yields an error. If not given,
// accepts any suffix (but one is required).
fn file_extension<P: AsRef<Path>>(path: P, expected_suffix: Option<&str>) -> Result<Extension> {
    let path = path.as_ref();
    let ext = path.extension().ok_or_else(|| {
        let _hold;
        let extension_msg = if let Some(expected_suffix) = expected_suffix {
            _hold = format!("{expected_suffix:?}");
            &_hold
        } else {
            "any extension"
        };
        anyhow!("missing file extension, expecting {extension_msg} or \"zstd\": {path:?}")
    })?;

    match ext.to_string_lossy().as_ref() {
        "zstd" => {
            if let Some(expected_suffix) = expected_suffix {
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
            } else {
                Ok(Extension::ZStd)
            }
        }
        ext_str => {
            if let Some(expected_suffix) = expected_suffix {
                if ext_str == expected_suffix {
                    Ok(Extension::Other)
                } else {
                    bail!(
                        "unknown file extension {ext:?}, expecting {expected_suffix:?} \
                     or \"zstd\": {path:?}"
                    )
                }
            } else {
                Ok(Extension::Other)
            }
        }
    }
}

#[test]
fn t_file_extension() {
    use Extension::*;
    let ok = |a: &str, b: &'static str| {
        file_extension(a, Some(b)).expect("test call should not give an error")
    };
    let err = |a: &str, b: &'static str| {
        file_extension(a, Some(b))
            .err()
            .expect("test call should give an error")
            .to_string()
    };
    assert_eq!(ok("foo.x", "x"), Other);
    assert_eq!(ok("foo.x.zstd", "x"), ZStd);
    assert_eq!(ok("foo.z.x", "x"), Other);
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

pub trait SendRead: Read + Send {}

impl SendRead for StreamingDecoder<std::fs::File, FrameDecoder> {}
impl SendRead for ChildStdout {}
impl SendRead for File {}

/// Transparently decompress zstd files if they have a .zstd suffix;
/// after that, expecting the `expected_suffix` (which must be given
/// *without* a leading dot) if given.
pub fn decompressed_file(path: &Path, expected_suffix: Option<&str>) -> Result<Box<dyn SendRead>> {
    let ext = file_extension(path, expected_suffix)?;

    let file_open = || File::open(path).with_context(|| anyhow!("opening file {path:?}"));

    match ext {
        Extension::ZStd => {
            if USING_EXTERNAL_TOOL {
                let mut c = Command::new("zstd");
                let args: Vec<OsString> = vec!["-dcf".into(), "--".into(), path.into()];
                c.args(args);
                c.stdout(Stdio::piped());
                let child = c.spawn().map_err(ctx!("spawning command {c:?}"))?;
                Ok(Box::new(child.stdout.expect("present since configured")))
            } else {
                let input = file_open()?;
                Ok(Box::new(
                    StreamingDecoder::new(input).map_err(ctx!("zstd-decoding {path:?}"))?,
                ))
            }
        }
        Extension::Other => Ok(Box::new(file_open()?)),
    }
}

/// If quiet is false, lets messaging by the `zstd` tool show up on
/// stdout/err. If true, silences reporting output but captures error
/// messages and reports those in the resulting error.
pub fn compress_file(source_path: &Path, target_path: &Path, quiet: bool) -> Result<()> {
    let mut c = Command::new("zstd");
    if quiet {
        c.arg("--quiet");
        c.stdout(Stdio::piped());
        c.stderr(Stdio::piped());
    }
    let args: &[&OsStr] = &[
        "-o".as_ref(),
        // XX: is this argument position safe against option injection?
        target_path.as_ref(),
        "--".as_ref(),
        source_path.as_ref(),
    ];
    c.args(args);
    let output = c.output().map_err(ctx!("running command {c:?}"))?;
    let status = output.status;
    if status.success() {
        Ok(())
    } else {
        let outputs = if quiet {
            let mut outputs = String::from_utf8_lossy(&output.stdout).into_owned();
            outputs.push_str(&String::from_utf8_lossy(&output.stderr));
            format!("{outputs:?}")
        } else {
            "not captured".into()
        };
        bail!("running zstd {args:?}: {status} with outputs {outputs}")
    }
}
