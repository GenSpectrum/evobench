//! A value, or reference to a value from a value defined in a context
//!
//! Do not confuse with `RefOrOwned`: `RefOrOwned` is for Rust
//! references, this is for string references to some keyed collection.

use anyhow::{Result, anyhow};
use kstring::KString;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap, fmt::Debug, marker::PhantomData, ops::Deref};

/// A context for a `ValOrRef` to look up values in
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValOrRefContext<TD: ContextDesc, T>(BTreeMap<KString, T>, PhantomData<fn() -> TD>);

impl<TD: ContextDesc, T> Deref for ValOrRefContext<TD, T> {
    type Target = BTreeMap<KString, T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<TD: ContextDesc, T> From<BTreeMap<KString, T>> for ValOrRefContext<TD, T> {
    fn from(value: BTreeMap<KString, T>) -> Self {
        Self(value, PhantomData)
    }
}

/// Human-readable description for the context; provided statically so
/// that it can be shown at config deserialisation time
pub trait ContextDesc {
    fn context_desc() -> Cow<'static, str>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValOrRefInner<T> {
    Val(T),
    Ref(KString),
}

// Wrapper type to hide the enum to force access via
// `value_with_context`, to avoid accidental mis-use of the reference
// names, and also to carry the TargetDesc phantom type parameter.

/// Either a value directly, or a string naming an entry in a context
#[derive(Debug)]
pub struct ValOrRef<TD: ContextDesc, T> {
    inner: ValOrRefInner<T>,
    source: PhantomData<fn() -> TD>,
}

impl<TD: ContextDesc, T> From<ValOrRefInner<T>> for ValOrRef<TD, T> {
    fn from(inner: ValOrRefInner<T>) -> Self {
        Self {
            inner,
            source: PhantomData,
        }
    }
}

impl<TD: ContextDesc, T: Clone> Clone for ValOrRef<TD, T> {
    fn clone(&self) -> Self {
        let Self { inner, source: _ } = self;
        Self {
            inner: inner.clone(),
            source: PhantomData,
        }
    }
}

impl<TD: ContextDesc, T> ValOrRef<TD, T> {
    /// Returns a reference to the contained value, or the entry in
    /// the given map. Returns None if this is a Ref and the reference
    /// is not present in the map.
    pub fn get_value_with_context<'s>(
        &'s self,
        context: &'s ValOrRefContext<TD, T>,
    ) -> Option<&'s T> {
        match &self.inner {
            ValOrRefInner::Val(v) => Some(v),
            ValOrRefInner::Ref(r) => context.get(r),
        }
    }

    /// Same as `get_value_with_context` but returns an error if the
    /// reference cannot be resolved
    pub fn value_with_context<'s>(&'s self, context: &'s ValOrRefContext<TD, T>) -> Result<&'s T> {
        match &self.inner {
            ValOrRefInner::Val(v) => Ok(v),
            ValOrRefInner::Ref(r) => context.get(r).ok_or_else(|| {
                anyhow!(
                    "name {:?} is not present in {}",
                    r.as_str(),
                    TD::context_desc()
                )
            }),
        }
    }

    /// Map to contain the different stored type, if necessary
    pub fn into_try_map<U, E>(self, f: impl Fn(T) -> Result<U, E>) -> Result<ValOrRef<TD, U>, E> {
        Ok(ValOrRef {
            inner: match self.inner {
                ValOrRefInner::Val(v) => ValOrRefInner::Val(f(v)?),
                ValOrRefInner::Ref(r) => ValOrRefInner::Ref(r),
            },
            source: PhantomData,
        })
    }

    /// Map to contain the different stored type, if necessary
    pub fn try_map<U, E>(&self, f: impl Fn(&T) -> Result<U, E>) -> Result<ValOrRef<TD, U>, E> {
        Ok(ValOrRef {
            inner: match &self.inner {
                ValOrRefInner::Val(v) => ValOrRefInner::Val(f(v)?),
                ValOrRefInner::Ref(r) => ValOrRefInner::Ref(r.clone()),
            },
            source: PhantomData,
        })
    }
}

impl<'de, TD: ContextDesc, T: Deserialize<'de>> Deserialize<'de> for ValOrRef<TD, T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let inner = ValOrRefInner::deserialize(deserializer)?;
        Ok(Self {
            inner,
            source: PhantomData,
        })
    }
}

impl<TD: ContextDesc, T: Serialize> Serialize for ValOrRef<TD, T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.inner.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct U32Desc;
    impl ContextDesc for U32Desc {
        fn context_desc() -> Cow<'static, str> {
            "hi".into()
        }
    }

    #[test]
    fn t_() {
        let inner = ValOrRefInner::<u32>::Val(120);
        let _whole: ValOrRef<U32Desc, u32> = inner.into();
    }
}
