//! Stupid. Cow doesn't work for clonable types that don't have their
//! own borrows? I don't understand why. Just do my own now.

use std::{fmt::Display, ops::Deref};

#[derive(Clone, Debug)]
pub enum RefOrOwned<'t, T> {
    Ref(&'t T),
    Owned(T),
}

impl<'t, T> RefOrOwned<'t, T> {
    pub fn as_ref(&self) -> &T {
        match self {
            RefOrOwned::Ref(borrowed) => borrowed,
            RefOrOwned::Owned(owned) => owned,
        }
    }

    pub fn into_owned(self) -> T
    where
        T: Clone,
    {
        match self {
            RefOrOwned::Ref(borrowed) => borrowed.clone(),
            RefOrOwned::Owned(owned) => owned,
        }
    }
}

impl<'t, T> From<&'t T> for RefOrOwned<'t, T> {
    fn from(value: &'t T) -> Self {
        Self::Ref(value)
    }
}

impl<'t, T> From<T> for RefOrOwned<'t, T> {
    fn from(value: T) -> Self {
        Self::Owned(value)
    }
}

impl<'t, T> Deref for RefOrOwned<'t, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'t, T: Display> Display for RefOrOwned<'t, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_ref().fmt(f)
    }
}
