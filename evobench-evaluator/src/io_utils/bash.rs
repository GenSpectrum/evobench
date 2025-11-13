use std::borrow::Cow;

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

pub fn bash_string_from_program_and_args(
    cmd: impl AsRef<str>,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> String {
    let mut cmd = cmd.as_ref().to_owned();
    for arg in args {
        cmd.push_str(" ");
        cmd.push_str(&*bash_string_literal(arg.as_ref()));
    }
    cmd
}

pub fn bash_export_variable_string(name: &str, val: &str, prefix: &str, suffix: &str) -> String {
    format!(
        "{prefix}export {}={}{suffix}",
        bash_string_literal(name),
        bash_string_literal(val)
    )
}
