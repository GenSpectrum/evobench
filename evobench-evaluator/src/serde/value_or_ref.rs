//! Do not confuse with `RefOrOwned`: `RefOrOwned` is for Rust
//! references, this is for string references to some keyed collection.

use anyhow::{anyhow, Result};
use kstring::KString;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap, fmt::Debug, marker::PhantomData};

pub trait ValueOrRefTarget {
    fn target_desc() -> Cow<'static, str>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ValueOrRefInner<T> {
    Value(T),
    Ref(KString),
}

/// A string naming an entry in another place, or holding a value directly.
// Hiding the enum to force access via `value_with_backing`, to avoid
// accidental mis-use of the reference names.
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueOrRef<RefTarget: ValueOrRefTarget, T> {
    #[serde(flatten)]
    inner: ValueOrRefInner<T>,
    #[serde(skip)]
    source: PhantomData<fn() -> RefTarget>,
}

impl<RefTarget: ValueOrRefTarget, T: Clone> Clone for ValueOrRef<RefTarget, T> {
    fn clone(&self) -> Self {
        let Self { inner, source: _ } = self;
        Self {
            inner: inner.clone(),
            source: PhantomData,
        }
    }
}

impl<RefTarget: ValueOrRefTarget, T> ValueOrRef<RefTarget, T> {
    /// Returns a reference to the contained value, or the entry in
    /// the given map. Returns None if this is a Ref and the reference
    /// is not present in the map.
    pub fn get_value_with_backing<'s>(&'s self, map: &'s BTreeMap<KString, T>) -> Option<&'s T> {
        match &self.inner {
            ValueOrRefInner::Value(v) => Some(v),
            ValueOrRefInner::Ref(r) => map.get(r),
        }
    }

    /// Same as `get_value_with_backing` but returns an error if the
    /// reference cannot be resolved
    pub fn value_with_backing<'s>(&'s self, map: &'s BTreeMap<KString, T>) -> Result<&'s T> {
        match &self.inner {
            ValueOrRefInner::Value(v) => Ok(v),
            ValueOrRefInner::Ref(r) => map.get(r).ok_or_else(|| {
                anyhow!(
                    "unknown name {:?} in {}",
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
    ) -> Result<ValueOrRef<RefTarget, U>, E> {
        Ok(ValueOrRef {
            inner: match self.inner {
                ValueOrRefInner::Value(v) => ValueOrRefInner::Value(f(v)?),
                ValueOrRefInner::Ref(r) => ValueOrRefInner::Ref(r),
            },
            source: PhantomData,
        })
    }

    /// Map to contain the different stored type, if necessary
    pub fn try_map<U, E>(
        &self,
        f: impl Fn(&T) -> Result<U, E>,
    ) -> Result<ValueOrRef<RefTarget, U>, E> {
        Ok(ValueOrRef {
            inner: match &self.inner {
                ValueOrRefInner::Value(v) => ValueOrRefInner::Value(f(v)?),
                ValueOrRefInner::Ref(r) => ValueOrRefInner::Ref(r.clone()),
            },
            source: PhantomData,
        })
    }
}
