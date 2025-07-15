use std::{fmt::Display, str::FromStr};

use serde::de::Visitor;

/// A unicode file name, not path, i.e. not contain '/', '\n', or '\0'
/// and must not be ".", "..", or "".
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, serde::Serialize, Hash)]
pub struct ProperFilename(String);

impl ProperFilename {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ProperFilename {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'t> From<&'t ProperFilename> for &'t str {
    fn from(value: &'t ProperFilename) -> Self {
        value.as_str()
    }
}

impl Display for ProperFilename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

const ERR_MSG: &str = "a file name (not path), must not contain '/', '\\n', '\\0', \
     and must not be \".\", \"..\", the empty string, or longer than 255 bytes";
// XX Windows will be different than "bytes" and 255.

impl FromStr for ProperFilename {
    type Err = &'static str;

    fn from_str(v: &str) -> Result<Self, Self::Err> {
        if v == ""
            || v == "."
            || v == ".."
            || v.contains('/')
            || v.contains('\n')
            || v.contains('\0')
            || v.as_bytes().len() > 255
        {
            return Err(ERR_MSG);
        }
        Ok(ProperFilename(v.to_owned()))
    }
}

struct FilenameVisitor;
impl<'de> Visitor<'de> for FilenameVisitor {
    type Value = ProperFilename;

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

impl<'de> serde::Deserialize<'de> for ProperFilename {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(FilenameVisitor)
    }
}
