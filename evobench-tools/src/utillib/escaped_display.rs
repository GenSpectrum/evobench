//! Trait and temporary wrapper around strings and paths to `Display`
//! them escaped.
//!
//! The motivation is to avoid using `{:?}` in format strings--those
//! are dangerous in that this is just *really* "the debug" format,
//! and if e.g. a Path is replaced with a complex type, the complex'
//! type internals are dumped instead of just an escaped path. I've
//! run into this enough times now to be tired of it.

use std::{
    fmt::{Debug, Display},
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct DebugForDisplay<T: Debug>(pub T);

impl<T: Debug> Display for DebugForDisplay<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// Also implement Debug so that DebugForDisplay is usable in
/// e.g. tuples that are to be shown via `:?`
impl<T: Debug> Debug for DebugForDisplay<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

pub trait AsEscapedString {
    type ViewableType<'t>
    where
        Self: 't;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s>;
}

impl<'u> AsEscapedString for &'u str {
    type ViewableType<'t>
        = DebugForDisplay<&'t str>
    where
        Self: 't;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s> {
        DebugForDisplay(*self)
    }
}

impl AsEscapedString for String {
    type ViewableType<'t> = DebugForDisplay<&'t str>;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s> {
        DebugForDisplay(&**self)
    }
}

impl<'u> AsEscapedString for &'u Path {
    type ViewableType<'t>
        = DebugForDisplay<&'t Path>
    where
        Self: 't;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s> {
        DebugForDisplay(&**self)
    }
}

impl<'u> AsEscapedString for &'u Arc<Path> {
    type ViewableType<'t>
        = DebugForDisplay<&'t Path>
    where
        Self: 't;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s> {
        DebugForDisplay(&**self)
    }
}

impl AsEscapedString for PathBuf {
    type ViewableType<'t> = DebugForDisplay<&'t Path>;

    fn as_escaped_string<'s>(&'s self) -> Self::ViewableType<'s> {
        DebugForDisplay(&**self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_s() {
        let s1 = "hello world";
        assert_eq!(format!("<{}>", s1.as_escaped_string()), "<\"hello world\">");
        let s2 = s1.to_owned() + " and so on";
        assert_eq!(
            format!("<{}>", s2.as_escaped_string()),
            "<\"hello world and so on\">"
        );
    }

    #[test]
    fn t_p() {
        let p1: &Path = "hi there".as_ref();
        assert_eq!(format!("<{}>", p1.as_escaped_string()), "<\"hi there\">");
        let p2 = p1.join("and more");
        assert_eq!(
            format!("<{}>", p2.as_escaped_string()),
            "<\"hi there/and more\">"
        );
    }
}
