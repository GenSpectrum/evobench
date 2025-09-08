/// Like `ProperFilename` but also doesn't allow a suffix
// Here subtyping would really help. How to achievethat otherwise?
// Deref doesn't do it, actually. XX Could use newtype utility crate
// though?
use std::{path::Path, str::FromStr};

use serde::de::Visitor;

use super::proper_filename::is_proper_filename;

pub fn has_extension(v: &str) -> bool {
    let p: &Path = v.as_ref();
    p.extension().is_some()
}

pub fn is_proper_dirname(v: &str) -> bool {
    is_proper_filename(v) && !has_extension(v)
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct ProperDirname(String);

impl ProperDirname {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ProperDirname {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'t> From<&'t ProperDirname> for &'t str {
    fn from(value: &'t ProperDirname) -> Self {
        value.as_str()
    }
}

const ERR_MSG: &str = "a file name (not path), must not contain '/', '\\n', '\\0', \
     and must not be \".\", \"..\", the empty string, or longer than 255 bytes";
// XX Windows will be different than "bytes" and 255.

impl FromStr for ProperDirname {
    type Err = &'static str;

    fn from_str(v: &str) -> Result<Self, Self::Err> {
        if !is_proper_dirname(v) {
            return Err(ERR_MSG);
        }
        Ok(ProperDirname(v.to_owned()))
    }
}

struct FilenameVisitor;
impl<'de> Visitor<'de> for FilenameVisitor {
    type Value = ProperDirname;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(ERR_MSG)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for ProperDirname {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(FilenameVisitor)
    }
}

impl serde::Serialize for ProperDirname {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}
