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
    pub fn into_owned(self) -> Box<[T]>
    where
        T: Clone,
    {
        match self {
            SliceOrBox::Slice(borrowed) => borrowed.to_owned().into(),
            SliceOrBox::Box(owned) => owned,
        }
    }
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
