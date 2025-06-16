//! Try to use data from the OS (OsString) as strings, if possible,
//! fall back to the original data (lossless) if not possible.

use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, serde::Serialize, serde::Deserialize)]
pub enum SafeString {
    OsString(OsString),
    String(String),
}

impl From<OsString> for SafeString {
    fn from(value: OsString) -> Self {
        match value.into_string() {
            Ok(s) => Self::String(s),
            Err(value) => Self::OsString(value),
        }
    }
}

impl From<String> for SafeString {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<SafeString> for OsString {
    fn from(value: SafeString) -> Self {
        match value {
            SafeString::OsString(os_string) => os_string,
            SafeString::String(s) => OsString::from(s),
        }
    }
}

#[derive(Debug)]
pub struct NeverError;

impl std::error::Error for NeverError {}

impl Display for NeverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this error never happens")
    }
}

// Required for Clap. Hmm, but does it still handle byte fallbacks??
impl FromStr for SafeString {
    type Err = NeverError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.to_owned().into())
    }
}

impl AsRef<OsStr> for SafeString {
    fn as_ref(&self) -> &OsStr {
        match self {
            SafeString::OsString(os_string) => os_string.as_ref(),
            SafeString::String(s) => s.as_ref(),
        }
    }
}

// XX add *custom* Debug, custom eq and ord, as well as conversions
// from references?
