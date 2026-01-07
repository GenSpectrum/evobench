use std::{ffi::OsStr, fmt::Display, marker::PhantomData, ops::Deref, str::FromStr};

use anyhow::{Result, bail};
use kstring::KString;
use serde::de::Visitor;

use crate::utillib::type_name_short::type_name_short;

pub trait AllowEnvVar {
    /// Max allowed variable name length in UTF-8 bytes
    const MAX_ENV_VAR_NAME_LEN: usize;
    fn allow_env_var(s: &str) -> bool;
    fn expecting() -> String;
}

#[derive(Debug)]
pub struct AllowedEnvVar<A: AllowEnvVar>(KString, PhantomData<fn() -> A>);

impl<A: AllowEnvVar> Clone for AllowedEnvVar<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<A: AllowEnvVar> Deref for AllowedEnvVar<A> {
    type Target = KString;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<A: AllowEnvVar> AsRef<str> for AllowedEnvVar<A> {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl<A: AllowEnvVar> AsRef<OsStr> for AllowedEnvVar<A> {
    fn as_ref(&self) -> &OsStr {
        self.0.as_str().as_ref()
    }
}

impl<A: AllowEnvVar> PartialEq for AllowedEnvVar<A> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl<A: AllowEnvVar> Eq for AllowedEnvVar<A> {}

impl<A: AllowEnvVar> PartialOrd for AllowedEnvVar<A> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}
impl<A: AllowEnvVar> Ord for AllowedEnvVar<A> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<A: AllowEnvVar> FromStr for AllowedEnvVar<A> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains('\0') {
            bail!("null characters are not allowed in environment variable names")
        }
        if s.contains('=') {
            bail!("'=' characters are not allowed in environment variable names")
        }
        if s.len() > A::MAX_ENV_VAR_NAME_LEN {
            bail!(
                "{} environment variable names must not be longer than {} bytes",
                type_name_short::<A>(),
                A::MAX_ENV_VAR_NAME_LEN
            )
        }
        if A::allow_env_var(s) {
            Ok(Self(KString::from_ref(s), PhantomData))
        } else {
            bail!(
                "{} env variable {s:?} is reserved, expecting {}",
                type_name_short::<A>(),
                A::expecting()
            )
        }
    }
}

impl<A: AllowEnvVar> Display for AllowedEnvVar<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

struct OurVisitor<A: AllowEnvVar>(PhantomData<fn() -> A>);
impl<'de, A: AllowEnvVar> Visitor<'de> for OurVisitor<A> {
    type Value = AllowedEnvVar<A>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(&A::expecting())
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        AllowedEnvVar::<A>::from_str(v).map_err(E::custom)
    }
}

impl<'de, A: AllowEnvVar> serde::Deserialize<'de> for AllowedEnvVar<A> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(OurVisitor(PhantomData))
    }
}

impl<A: AllowEnvVar> serde::Serialize for AllowedEnvVar<A> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}
