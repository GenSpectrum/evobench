// (Didn't I have a lazy macro already?)

// std::cell::LazyCell is an unstable feature; but also want to have
// easy wrapper macros and usable `Result` handling, to achieve the
// latter do not implement `Deref` (since that would have to panic to
// unwrap the value from the Result). Given that methods are now
// unambiguous, make `force` and the other accessors normal methods.

use std::fmt;

pub enum Lazy<T, F: FnOnce() -> T> {
    Thunk(F),
    Poisoned,
    Value(T)
}

impl<T, F: FnOnce() -> T> Lazy<T, F> {
    pub fn new(f: F) -> Self {
        Self::Thunk(f)
    }
    pub fn into_inner(self) -> Result<T, F> {
        match self {
            Lazy::Value(v) => Ok(v),
            Lazy::Thunk(f) => Err(f),
            Lazy::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }
    pub fn force(&mut self) -> &T {
        match self {
            Lazy::Thunk(_) => {
                let mut thunk = Lazy::Poisoned;
                std::mem::swap(self, &mut thunk);
                match thunk {
                    Lazy::Thunk(t) => {
                        let mut new = Lazy::Value(t());
                        std::mem::swap(self, &mut new);
                        match self {
                            Lazy::Value(v) => v,
                            _ => panic!()
                        }
                    }
                    _ => panic!()
                }
            }
            Lazy::Value(v) => v,
            Lazy::Poisoned => panic!("Lazy instance has previously been poisoned")
        }
    }
}


/// Variant that does not store error resuls from the thunk in the
/// promise, so that Result or E do not need to support clone yet
/// errors can still be propagated. It is at odds with FnOnce,
/// though.
pub enum LazyResult<T, E, F: FnOnce() -> Result<T, E>> {
    Thunk(F),
    Poisoned,
    Value(T)
}

impl<T, E, F: FnOnce() -> Result<T, E>> LazyResult<T, E, F> {
    pub fn new(f: F) -> Self {
        Self::Thunk(f)
    }
    pub fn into_inner(self) -> Result<T, F> {
        match self {
            Self::Value(v) => Ok(v),
            Self::Thunk(f) => Err(f),
            Self::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }
    pub fn force(&mut self) -> Result<&T, E> {
        match self {
            Self::Thunk(_) => {
                let mut thunk = Self::Poisoned;
                std::mem::swap(self, &mut thunk);
                match thunk {
                    Self::Thunk(t) => {
                        let mut new: Self = Self::Value(t()?);
                        std::mem::swap(self, &mut new);
                        match self {
                            Self::Value(v) => Ok(v),
                            _ => panic!()
                        }
                    }
                    _ => panic!()
                }
            }
            Self::Value(v) => Ok(v),
            Self::Poisoned => panic!("Lazy instance has previously been poisoned")
        }
    }
}


impl<T: fmt::Debug, F: FnOnce() -> T> fmt::Debug for Lazy<T, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("Lazy");
        match self {
            Self::Thunk(_) => d.field(&format_args!("<uninit>")),
            Self::Poisoned => d.field(&format_args!("<poisoned>")),
            Self::Value(v) => d.field(v),
        };
        d.finish()
    }
}

impl<T: fmt::Debug, E, F: FnOnce() -> Result<T, E>> fmt::Debug for LazyResult<T, E, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("LazyResult");
        match self {
            Self::Thunk(_) => d.field(&format_args!("<uninit>")),
            Self::Poisoned => d.field(&format_args!("<poisoned>")),
            Self::Value(v) => d.field(v),
        };
        d.finish()
    }
}

// I'd prefer the move keyword to be in front of the { } but that
// doesn't work, lazy(move { }) is the next best (the { } are optional
// though, but then emacs indentation won't work right if using more
// than 1 expr).

/// Construct a `Lazy` value that runs the code body given to the
/// macro once on the first call to `force`. Moves the captured values
/// if a `move` keyword is given as the first item in the body. If the
/// body evaluates to a `Result`, better use `lazyresult!` instead
/// because `Result` is generally not cloneable and thus `force()?`
/// won't work.
#[macro_export]
macro_rules! lazy {
    { move $($body:tt)* } => {
        dev_utils::lazy::Lazy::new(move || { $($body)* })
    };
    { $($body:tt)* } => {
        dev_utils::lazy::Lazy::new(|| { $($body)* })
    }
}

/// Same as `lazy!` but `force` evaluates to a new `Result` each time,
/// allowing `force()?` to work seamlessly; but since, currently,
/// `force` consumes the captured closure when called the first time,
/// your code has to avoid calling `force` again if it got an error
/// (this is relying on the borrow checker being smart enough). Only
/// the first `force` call can fail, subsequent ones always return Ok.
#[macro_export]
macro_rules! lazyresult {
    { move $($body:tt)* } => {
        dev_utils::lazy::LazyResult::new(move || { $($body)* })
    };
    { $($body:tt)* } => {
        dev_utils::lazy::LazyResult::new(|| { $($body)* })
    }
}

