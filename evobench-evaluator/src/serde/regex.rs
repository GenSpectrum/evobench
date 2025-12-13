use std::{
    fmt::{Debug, Display},
    ops::Deref,
    str::FromStr,
};

use kstring::KString;
use regex::Regex;
use serde::de::Visitor;

#[derive(Clone)]
pub struct SerializableRegex {
    s: KString,
    r: Regex,
}

impl Debug for SerializableRegex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SerializableRegex").field(&self.s).finish()
    }
}

impl Display for SerializableRegex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.s.as_str())
    }
}

impl PartialEq for SerializableRegex {
    fn eq(&self, other: &Self) -> bool {
        self.s == other.s
    }
}

impl FromStr for SerializableRegex {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let r = Regex::new(s)?;
        Ok(Self {
            s: KString::from_ref(s),
            r,
        })
    }
}

impl Deref for SerializableRegex {
    type Target = Regex;

    fn deref(&self) -> &Self::Target {
        &self.r
    }
}

impl AsRef<str> for SerializableRegex {
    fn as_ref(&self) -> &str {
        &self.s
    }
}

impl AsRef<Regex> for SerializableRegex {
    fn as_ref(&self) -> &Regex {
        &self.r
    }
}

struct OurVisitor;
impl<'de> Visitor<'de> for OurVisitor {
    type Value = SerializableRegex;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(
            "a string representing a regular expression as supported \
             by the `regex` crate",
        )
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse().map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for SerializableRegex {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(OurVisitor)
    }
}

impl serde::Serialize for SerializableRegex {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.s.as_str())
    }
}
