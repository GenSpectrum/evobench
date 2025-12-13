use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Result;
use serde::de::Visitor;

use crate::utillib::path_resolve_home::path_resolve_home;

/// Accept paths starting with `~/` to mean going from the user's home
/// directory. Does not currently support `~user/`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct TildePath<P: AsRef<Path>>(P);

impl<P: AsRef<Path>> TildePath<P> {
    /// Change paths starting with `~/` to replace the `~` with the user's
    /// home directory. XX Careful: if path is not representable as unicode
    /// string, no expansion is attempted!
    pub fn resolve(&self) -> Result<PathBuf> {
        path_resolve_home(self.0.as_ref())
    }
}

impl FromStr for TildePath<PathBuf> {
    type Err = core::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(PathBuf::from_str(s)?))
    }
}

struct OurVisitor;
impl<'de> Visitor<'de> for OurVisitor {
    type Value = TildePath<PathBuf>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("anything, this is infallible")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for TildePath<PathBuf> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(OurVisitor)
    }
}

impl serde::Serialize for TildePath<PathBuf> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(s) = self.0.to_str() {
            serializer.serialize_str(s)
        } else {
            Err(serde::ser::Error::custom("path contains invalid UTF-8"))
        }
    }
}
