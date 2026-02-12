// (Didn't I have a lazy macro already?)

// std::cell::LazyCell is an unstable feature; but also want to have
// easy wrapper macros and usable `Result` handling, to achieve the
// latter do not implement `Deref` (since that would have to panic to
// unwrap the value from the Result). Given that methods are now
// unambiguous, make `force` and the other accessors normal methods.

// Want to avoid `force` to require &mut, hence need a cell type.
// Because something needs to be stored from the start, OnceCell
// doesn't work. RefCell would work but stores an additional flag. Go
// unsafe.

use std::{cell::UnsafeCell, fmt};

/// TODO: update to use UnsafeCell like LazyResult
pub enum Lazy<T, F: FnOnce() -> T> {
    Thunk(F),
    Poisoned,
    Value(T),
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

    /// Forces evaluation if not already evaluated
    pub fn into_value(self) -> T {
        match self.into_inner() {
            Ok(v) => v,
            Err(f) => f(),
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
                            _ => panic!(),
                        }
                    }
                    _ => panic!(),
                }
            }
            Lazy::Value(v) => v,
            Lazy::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }

    pub fn force_mut(&mut self) -> &mut T {
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
                            _ => panic!(),
                        }
                    }
                    _ => panic!(),
                }
            }
            Lazy::Value(v) => v,
            Lazy::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }
}

/// Variant that does not store error resuls from the thunk in the
/// promise, so that Result or E do not need to support clone yet
/// errors can still be propagated. It is at odds with FnOnce,
/// though.
enum InnerLazyResult<T, E, F: FnOnce() -> Result<T, E>> {
    Thunk(F),
    Poisoned,
    Value(T),
}

pub struct LazyResult<T, E, F: FnOnce() -> Result<T, E>>(UnsafeCell<InnerLazyResult<T, E, F>>);

impl<T, E, F: FnOnce() -> Result<T, E>> LazyResult<T, E, F> {
    pub fn new(f: F) -> Self {
        Self(UnsafeCell::new(InnerLazyResult::Thunk(f)))
    }

    pub fn into_inner(self) -> Result<T, F> {
        match self.0.into_inner() {
            InnerLazyResult::Value(v) => Ok(v),
            InnerLazyResult::Thunk(f) => Err(f),
            InnerLazyResult::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }

    /// Forces evaluation if not already evaluated
    pub fn into_value(self) -> Result<T, E> {
        match self.into_inner() {
            Ok(v) => Ok(v),
            Err(f) => f(),
        }
    }

    pub fn force(&self) -> Result<&T, E> {
        let rf = unsafe {
            // Safe because access is private, Self is not Sync or
            // Send, it is initialized, and swapped. Drop still runs
            // on the UnsafeCell contents, as verified by the test
            // further down.
            &mut *self.0.get()
        };
        match rf {
            InnerLazyResult::Thunk(_) => {
                let mut thunk = InnerLazyResult::Poisoned;
                std::mem::swap(rf, &mut thunk);
                match thunk {
                    InnerLazyResult::Thunk(t) => {
                        let mut new = InnerLazyResult::Value(t()?);
                        std::mem::swap(rf, &mut new);
                        match rf {
                            InnerLazyResult::Value(v) => Ok(v),
                            _ => panic!(),
                        }
                    }
                    _ => panic!(),
                }
            }
            InnerLazyResult::Value(v) => Ok(v),
            InnerLazyResult::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }

    pub fn force_mut(&mut self) -> Result<&mut T, E> {
        let rf = self.0.get_mut();
        match rf {
            InnerLazyResult::Thunk(_) => {
                let mut thunk = InnerLazyResult::Poisoned;
                std::mem::swap(rf, &mut thunk);
                match thunk {
                    InnerLazyResult::Thunk(t) => {
                        let mut new = InnerLazyResult::Value(t()?);
                        std::mem::swap(rf, &mut new);
                        match rf {
                            InnerLazyResult::Value(v) => Ok(v),
                            _ => panic!(),
                        }
                    }
                    _ => panic!(),
                }
            }
            InnerLazyResult::Value(v) => Ok(v),
            InnerLazyResult::Poisoned => panic!("Lazy instance has previously been poisoned"),
        }
    }
}

impl<T: fmt::Debug, F: FnOnce() -> T> fmt::Debug for Lazy<T, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("Lazy");
        match self {
            Self::Thunk(_) => d.field(&format_args!("<unforced>")),
            Self::Poisoned => d.field(&format_args!("<poisoned>")),
            Self::Value(v) => d.field(v),
        };
        d.finish()
    }
}

impl<T: fmt::Debug, E, F: FnOnce() -> Result<T, E>> fmt::Debug for LazyResult<T, E, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_tuple("LazyResult");
        let rf = unsafe {
            // See notes above, also, self is not called recursively
            &*self.0.get()
        };
        match rf {
            InnerLazyResult::Thunk(_) => d.field(&format_args!("<unforced>")),
            InnerLazyResult::Poisoned => d.field(&format_args!("<poisoned>")),
            InnerLazyResult::Value(v) => d.field(v),
        };
        d.finish()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use anyhow::Result;

    use crate::lazyresult;

    struct Foo<'t>(&'t Cell<i32>);

    impl<'t> Drop for Foo<'t> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn t_() -> Result<()> {
        let counter = Cell::new(0);
        let l = lazyresult! {
            anyhow::Ok(Foo(&counter))
        };
        // This must not compile:
        // {
        //     let (write, read) = std::sync::mpsc::channel();
        //     let other_thread = std::thread::spawn(|| {
        //         for msg in read {
        //             dbg!("uh");
        //         }
        //     });
        //     write.send(l);
        // }
        assert_eq!(counter.get(), 0);
        let foo = l.force()?;
        assert_eq!(foo.0.get(), 0);
        drop(l);
        assert_eq!(counter.get(), 1);

        Ok(())
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
        $crate::lazy::Lazy::new(move || { $($body)* })
    };
    { $($body:tt)* } => {
        $crate::lazy::Lazy::new(|| { $($body)* })
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
        $crate::utillib::lazy::LazyResult::new(move || { $($body)* })
    };
    { $($body:tt)* } => {
        $crate::utillib::lazy::LazyResult::new(|| { $($body)* })
    }
}
