//! `Cow<'t, [T]>` requires `T` to implement `ToOwned`,
//! i.e. `to_owned()` wants to make a deep copy,
//! apparently. `SliceOrBox` is just about the outermost
//! layer. (`RefOrOwned` only represents a reference or owned value of
//! the same type and hence does not work for this, either.)

// And a RefOrBox, same but without the explicit [ ], would not work
// because then the type would become unsized.

use std::ops::Deref;

#[derive(Clone, Debug)]
pub enum SliceOrBox<'t, T> {
    Slice(&'t [T]),
    Box(Box<[T]>),
}

impl<'t, T> SliceOrBox<'t, T> {
    /// Into the inner owned representation, or convert that to
    /// owned. Removes the wrapping.
    pub fn into_owned(self) -> Box<[T]>
    where
        T: Clone,
    {
        match self {
            SliceOrBox::Slice(borrowed) => borrowed.to_owned().into(),
            SliceOrBox::Box(owned) => owned,
        }
    }

    // pub fn to_owned(&self) -> SliceOrBox<'static, T> {
    // ..  because ToOwned requires Borrow. But, into_owned and the wrap again is fine for now?
    // }
}

impl<'t, T> AsRef<[T]> for SliceOrBox<'t, T> {
    fn as_ref(&self) -> &[T] {
        match self {
            SliceOrBox::Slice(r) => r,
            SliceOrBox::Box(r) => r,
        }
    }
}

impl<'t, T> From<&'t [T]> for SliceOrBox<'t, T> {
    fn from(value: &'t [T]) -> Self {
        Self::Slice(value)
    }
}

impl<'t, T> From<Box<[T]>> for SliceOrBox<'t, T> {
    fn from(value: Box<[T]>) -> Self {
        Self::Box(value)
    }
}

impl<'t, T, const N: usize> From<[T; N]> for SliceOrBox<'t, T> {
    fn from(value: [T; N]) -> Self {
        Self::Box(value.into())
    }
}

impl<'t, T> From<Vec<T>> for SliceOrBox<'t, T> {
    fn from(value: Vec<T>) -> Self {
        Self::Box(value.into())
    }
}

impl<'t, T> Deref for SliceOrBox<'t, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

// Need these so that Option<SliceOrBox<..>> can be compared, right?
// "Why do I have to do this when Deref exists??"

impl<'t, T: PartialEq> PartialEq for SliceOrBox<'t, T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl<'t, T: Eq> Eq for SliceOrBox<'t, T> {}

impl<'t, T: PartialOrd> PartialOrd for SliceOrBox<'t, T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_ref().partial_cmp(other.as_ref())
    }
}

impl<'t, T: Ord> Ord for SliceOrBox<'t, T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}
