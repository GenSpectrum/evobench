//! Do not confuse with `RefOrOwned`: `RefOrOwned` is for Rust
//! references, this is for string references to some keyed collection.

use anyhow::{Result, anyhow};
use kstring::KString;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap, fmt::Debug, marker::PhantomData};

pub trait ValOrRefTarget {
    fn target_desc() -> Cow<'static, str>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValOrRefInner<T> {
    Val(T),
    Ref(KString),
}

/// A string naming an entry in another place, or holding a value directly.
// Hiding the enum to force access via `value_with_backing`, to avoid
// accidental mis-use of the reference names. Also need the wrapper to
// allow the phantom type parameter.
#[derive(Debug)]
pub struct ValOrRef<RefTarget: ValOrRefTarget, T> {
    inner: ValOrRefInner<T>,
    source: PhantomData<fn() -> RefTarget>,
}

impl<RefTarget: ValOrRefTarget, T> From<ValOrRefInner<T>> for ValOrRef<RefTarget, T> {
    fn from(inner: ValOrRefInner<T>) -> Self {
        Self {
            inner,
            source: PhantomData,
        }
    }
}

impl<RefTarget: ValOrRefTarget, T: Clone> Clone for ValOrRef<RefTarget, T> {
    fn clone(&self) -> Self {
        let Self { inner, source: _ } = self;
        Self {
            inner: inner.clone(),
            source: PhantomData,
        }
    }
}

impl<RefTarget: ValOrRefTarget, T> ValOrRef<RefTarget, T> {
    /// Returns a reference to the contained value, or the entry in
    /// the given map. Returns None if this is a Ref and the reference
    /// is not present in the map.
    pub fn get_value_with_backing<'s>(&'s self, map: &'s BTreeMap<KString, T>) -> Option<&'s T> {
        match &self.inner {
            ValOrRefInner::Val(v) => Some(v),
            ValOrRefInner::Ref(r) => map.get(r),
        }
    }

    /// Same as `get_value_with_backing` but returns an error if the
    /// reference cannot be resolved
    pub fn value_with_backing<'s>(&'s self, map: &'s BTreeMap<KString, T>) -> Result<&'s T> {
        match &self.inner {
            ValOrRefInner::Val(v) => Ok(v),
            ValOrRefInner::Ref(r) => map.get(r).ok_or_else(|| {
                anyhow!(
                    "name {:?} is not present in {}",
                    r.as_str(),
                    RefTarget::target_desc()
                )
            }),
        }
    }

    /// Map to contain the different stored type, if necessary
    pub fn into_try_map<U, E>(
        self,
        f: impl Fn(T) -> Result<U, E>,
    ) -> Result<ValOrRef<RefTarget, U>, E> {
        Ok(ValOrRef {
            inner: match self.inner {
                ValOrRefInner::Val(v) => ValOrRefInner::Val(f(v)?),
                ValOrRefInner::Ref(r) => ValOrRefInner::Ref(r),
            },
            source: PhantomData,
        })
    }

    /// Map to contain the different stored type, if necessary
    pub fn try_map<U, E>(
        &self,
        f: impl Fn(&T) -> Result<U, E>,
    ) -> Result<ValOrRef<RefTarget, U>, E> {
        Ok(ValOrRef {
            inner: match &self.inner {
                ValOrRefInner::Val(v) => ValOrRefInner::Val(f(v)?),
                ValOrRefInner::Ref(r) => ValOrRefInner::Ref(r.clone()),
            },
            source: PhantomData,
        })
    }
}

impl<'de, RefTarget: ValOrRefTarget, T: Deserialize<'de>> Deserialize<'de>
    for ValOrRef<RefTarget, T>
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let inner = ValOrRefInner::deserialize(deserializer)?;
        Ok(Self {
            inner,
            source: PhantomData,
        })
    }
}

impl<RefTarget: ValOrRefTarget, T: Serialize> Serialize for ValOrRef<RefTarget, T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}
