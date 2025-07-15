use std::{fmt::Display, str::FromStr};

use serde::de::Visitor;

/// A unicode file name, not path, i.e. not contain '/', '\n', or '\0'
/// and must not be ".", "..", or "".
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
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
        // NOTE: somewhat relying on the string quoting here now: both
        // in `list` subcommand, and I guess also in "summary-..."
        // file names it's better to show the value explicitly as a
        // separate string.
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

impl serde::Serialize for ProperFilename {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Returns Some when actual do, None when it wasn't a proper
    // filename.
    fn t_round_trip_json(s: &str) -> Option<()> {
        let pfn = ProperFilename::from_str(s).ok()?;
        assert_eq!(pfn.as_str(), s);
        let v = serde_json::to_string(&pfn).expect("doesn't fail");
        dbg!(&v);
        let pfn2: ProperFilename = serde_json::from_str(&v).expect("doesn't fail either");
        assert_eq!(pfn.as_str(), pfn2.as_str());
        Some(())
    }

    fn t_round_trip_ron(s: &str) -> Option<()> {
        let pfn = ProperFilename::from_str(s).ok()?;
        assert_eq!(pfn.as_str(), s);
        let v = ron::to_string(&pfn).expect("doesn't fail");
        dbg!(&v);
        let pfn2: ProperFilename = ron::from_str(&v).expect("doesn't fail either");
        assert_eq!(pfn.as_str(), pfn2.as_str());
        Some(())
    }

    fn t_round_trip(s: &str) -> Option<()> {
        t_round_trip_json(s)?;
        t_round_trip_ron(s)
    }

    #[test]
    fn t_proper_filename() {
        let t = t_round_trip;
        assert!(t("foo").is_some());
        assert!(t("<bar>").is_some());
        assert!(t(" baz .. bla").is_some());
        assert!(t(" baz/ .. bla").is_none());
    }
}
