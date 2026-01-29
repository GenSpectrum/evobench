use std::{fmt::Display, str::FromStr};

use serde::de::Visitor;

use crate::{
    git::GitHash,
    serde::git_reference::{GitReference, GitReferenceError, check_git_reference_string},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct GitBranchName(String);

impl GitBranchName {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_ref_string_in_remote(&self, remote_name: &str) -> String {
        format!("remotes/{remote_name}/{}", self.as_str())
    }

    pub fn to_reference(&self) -> GitReference {
        self.to_string()
            .parse()
            .expect("already checked to satisfy check_git_reference_string")
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

impl From<GitBranchName> for String {
    fn from(value: GitBranchName) -> Self {
        value.0
    }
}

impl Display for GitBranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitBranchNameError {
    #[error("a Git branch name must not be a Git commit hash")]
    IsGitHash,
    #[error("a Git branch name must be a valid Git reference, but {0}")]
    GitReferenceError(#[from] GitReferenceError),
}

impl FromStr for GitBranchName {
    type Err = GitBranchNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        check_git_reference_string(s)?;
        if let Ok(_) = GitHash::from_str(s) {
            return Err(GitBranchNameError::IsGitHash);
        }
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
