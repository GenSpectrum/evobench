use std::{fmt::Display, str::FromStr};

use serde::de::Visitor;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct GitBranchName(String);

impl GitBranchName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for GitBranchName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'t> From<&'t GitBranchName> for &'t str {
    fn from(value: &'t GitBranchName) -> Self {
        value.as_str()
    }
}

impl Display for GitBranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for GitBranchName {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Err("a git branch name must be non-empty")?
        }
        if s.chars().any(|c| c.is_whitespace() || c == '/' || c == '.') {
            // XX other characters, too
            Err("a git branch name must not contain whitespace, '/', '.'")?
        }
        // Assume it's OK
        Ok(Self(s.into()))
    }
}

struct GitBranchNameVisitor;
impl<'de> Visitor<'de> for GitBranchNameVisitor {
    type Value = GitBranchName;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a Git branch name")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for GitBranchName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(GitBranchNameVisitor)
    }
}
