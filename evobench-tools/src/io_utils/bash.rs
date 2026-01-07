use std::{borrow::Cow, path::Path};

use anyhow::{Result, anyhow};
use itertools::Itertools;

// (Todo?: add randomized tests with these calling bash)
const CHARS_NOT_NEEDING_QUOTING: &str = "_:.-+,/=@[]^";

// Once again. Have a better one somewhere.
pub fn bash_string_literal(s: &str) -> Cow<'_, str> {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || CHARS_NOT_NEEDING_QUOTING.contains(c))
    {
        s.into()
    } else {
        let mut ss = String::new();
        ss.push('\'');
        for c in s.chars() {
            if c == '\'' {
                ss.push('\'');
                ss.push('\\');
                ss.push('\'');
                ss.push('\'');
            } else {
                ss.push(c);
            }
        }
        ss.push('\'');
        ss.into()
    }
}

pub fn bash_string_from_cmd(cmd: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    cmd.into_iter()
        .map(|s| bash_string_literal(s.as_ref()).to_string())
        .join(" ")
}

pub fn bash_string_from_program_string_and_args(
    cmd: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> String {
    let mut cmd = bash_string_literal(cmd.as_ref()).into_owned();
    for arg in args {
        cmd.push_str(" ");
        cmd.push_str(&*bash_string_literal(arg.as_ref()));
    }
    cmd
}

/// Gives an error if cmd cannot be decoded as unicode
pub fn bash_string_from_program_path_and_args(
    cmd: impl AsRef<Path>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<String> {
    let path = cmd.as_ref();
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("can't decode path as unicode string: {path:?}"))?;
    let mut cmd = bash_string_literal(path_str).into_owned();
    for arg in args {
        cmd.push_str(" ");
        cmd.push_str(&*bash_string_literal(arg.as_ref()));
    }
    Ok(cmd)
}

pub fn bash_export_variable_string(name: &str, val: &str, prefix: &str, suffix: &str) -> String {
    format!(
        "{prefix}export {}={}{suffix}",
        bash_string_literal(name),
        bash_string_literal(val)
    )
}
