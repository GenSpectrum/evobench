use std::borrow::Cow;

use itertools::Itertools;

// (Todo?: add randomized tests with these calling bash)
const CHARS_NOT_NEEDING_QUOTING: &str = "_:.-+,/=@[]^";

// Once again. Have a better one somewhere.
pub fn bash_string(s: &str) -> Cow<str> {
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

pub fn cmd_as_bash_string<S: AsRef<str>>(cmd: impl IntoIterator<Item = S>) -> String {
    cmd.into_iter()
        .map(|s| bash_string(s.as_ref()).to_string())
        .join(" ")
}
