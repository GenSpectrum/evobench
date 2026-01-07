use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

use anyhow::{bail, Result};
use serde::de::Visitor;

fn decode_hex_digit(b: u8) -> Result<u8> {
    if b >= b'0' && b <= b'9' {
        Ok(b - b'0')
    } else if b >= b'a' && b <= b'f' {
        Ok(b - b'a' + 10)
    } else if b >= b'A' && b <= b'F' {
        Ok(b - b'A' + 10)
    } else {
        bail!("byte is not a hex digit: {b}")
    }
}

fn decode_hex<const N: usize>(input: &[u8], output: &mut [u8; N]) -> Result<()> {
    let n2 = 2 * N;
    if input.len() != n2 {
        bail!(
            "wrong number of hex digits, expect {n2}, got {}",
            input.len()
        )
    }
    for i in 0..N {
        output[i] = decode_hex_digit(input[i * 2])? * 16 + decode_hex_digit(input[i * 2 + 1])?;
    }
    Ok(())
}

#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitHash([u8; 20]);

impl Debug for GitHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GitHash!(\"")?;
        Display::fmt(self, f)?;
        f.write_str("\")")
    }
}

#[macro_export]
macro_rules! GitHash {
    {$hash:expr} => { GitHash::try_from($hash.as_bytes()).unwrap() }
}

impl TryFrom<&[u8]> for GitHash {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        // let s = std::str::from_utf8(value)?;
        if value.len() != 40 {
            bail!(
                "not a git hash of 40 hex bytes: {:?}",
                String::from_utf8_lossy(value)
            )
        }
        let mut bytes = [0; 20];
        decode_hex(value, &mut bytes)?;
        Ok(Self(bytes))
    }
}

impl FromStr for GitHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.chars().count() != s.len() {
            bail!("not an ASCII string: {s:?}")
        }
        GitHash::try_from(s.as_bytes())
    }
}

impl Display for GitHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in self.0 {
            f.write_fmt(format_args!("{:1x}{:1x}", b / 16, b & 15))?;
        }
        Ok(())
    }
}

#[test]
fn t_githash() -> Result<()> {
    let s = "18fdd1625c4d98526736ea8e5047a4ca818de0b4";
    let h1 = GitHash::try_from(s.as_bytes())?;
    let h = GitHash!(s);
    assert_eq!(h1, h);
    assert_eq!(h.0[0], 0x18);
    assert_eq!(h.0[1], 0xfd);
    assert_eq!(h.0[2], 0xd1);
    assert_eq!(format!("{h}"), s);
    Ok(())
}

const ERR_MSG: &str = "a full hexadecimal Git hash";

struct GitHashVisitor;
impl<'de> Visitor<'de> for GitHashVisitor {
    type Value = GitHash;

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

impl<'de> serde::Deserialize<'de> for GitHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(GitHashVisitor)
    }
}

impl serde::Serialize for GitHash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
