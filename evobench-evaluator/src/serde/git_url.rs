use std::{fmt::Display, str::FromStr};

use anyhow::{anyhow, bail};
use serde::de::Visitor;

use crate::utillib::path_resolve_home::path_resolve_home;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, serde::Serialize)]
pub struct GitUrl(String);

impl GitUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for GitUrl {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'t> From<&'t GitUrl> for &'t str {
    fn from(value: &'t GitUrl) -> Self {
        value.as_str()
    }
}

impl Display for GitUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

const ERR_MSG: &str = "a URL compatible with Git";

impl FromStr for GitUrl {
    type Err = anyhow::Error;

    fn from_str(v: &str) -> Result<Self, Self::Err> {
        let ok = Ok(GitUrl(v.to_owned()));

        if let Some(rest) = v.strip_prefix("https://") {
            if let Some((domain, other)) = rest.split_once('/') {
                if domain.is_empty() {
                    bail!("domain is empty")
                }
                if other.is_empty() {
                    bail!("part after domain is empty")
                }
            } else {
                bail!("expect a '/' between domain and location part")
            }
            return ok;
        }

        // XX is this correct, was it git:// ? All the rest the same or ?
        if let Some(rest) = v.strip_prefix("git://") {
            if let Some((domain, other)) = rest.split_once('/') {
                if domain.is_empty() {
                    bail!("domain is empty")
                }
                if other.is_empty() {
                    bail!("part after domain is empty")
                }
            } else {
                bail!("expect a '/' between domain and location part")
            }
            return ok;
        }

        if let Some(rest) = v.strip_prefix("file://") {
            if rest.is_empty() {
                bail!("empty file path given")
            }
            return ok;
        }

        if v.starts_with("/") || v.starts_with("../") {
            // OK?
            return ok;
        }

        if v.starts_with("~/") {
            let path = path_resolve_home(v.as_ref())?;
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow!("path {path:?} can't be represented as unicode string"))?;
            return Ok(GitUrl(path_str.to_owned()));
        }

        if let Some((user, rest)) = v.split_once('@') {
            if user.is_empty() {
                bail!("user is empty")
            }
            if let Some((domain, _path)) = rest.split_once(':') {
                if domain.is_empty() {
                    bail!("domain is empty")
                }
                // I guess path *can* be empty, if the home dir is the repo.
            } else {
                bail!("missing ':' in ssh based Git URL")
            }
            return ok;
        }

        bail!("no match for any kind of Git url known to this code")
    }
}

struct GitUrlVisitor;
impl<'de> Visitor<'de> for GitUrlVisitor {
    type Value = GitUrl;

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

impl<'de> serde::Deserialize<'de> for GitUrl {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(GitUrlVisitor)
    }
}
