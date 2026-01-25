use std::{fmt::Display, str::FromStr};

use serde::de::Visitor;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct GitReference(String);

impl GitReference {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for GitReference {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'t> From<&'t GitReference> for &'t str {
    fn from(value: &'t GitReference) -> Self {
        value.as_str()
    }
}

impl Display for GitReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for GitReference {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Err("a git reference string must be non-empty")?
        }
        if s.chars().any(|c| c.is_whitespace() || c == '/' || c == '.') {
            // XX other characters, too
            Err("a git reference string must not contain whitespace, '/', '.'")?
        }
        // Assume it's OK
        Ok(Self(s.into()))
    }
}

struct GitReferenceVisitor;
impl<'de> Visitor<'de> for GitReferenceVisitor {
    type Value = GitReference;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a Git reference string")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for GitReference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(GitReferenceVisitor)
    }
}
