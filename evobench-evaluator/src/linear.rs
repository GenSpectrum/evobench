//! Linear types for Rust--based on a hack, thus with some drawbacks:
//! `std::mem::leak` by-passes the linearity, compiler error messages
//! are not good (but might be good enough with another hack), and may
//! not be very solid (breaks if one wanted to add a `Debug`
//! implementation to the token type).

//! The "linear types" here are just types that have a `Drop`
//! implementation that is forbidden to be used (at compile time
//! already, by default). Only token types are supported: embedding
//! such a token type inside a larger data structure makes the larger
//! data structure linear, too. It then has to provide a moving method
//! or function that throws away the linear token by calling `bury()`
//! on it. With that approach, no wrapper around the containing data
//! structure is needed, which might be cleaner?

//! The idea and partially code came from
//! https://jack.wrenn.fyi/blog/undroppable/ and
//! https://geo-ant.github.io/blog/2024/rust-linear-types-use-once/

//! The only way to deal with a Linear instance is to call `bury()`,
//! or directly `std::mem::forget` on it. But it is recommended to
//! only use `bury()`, to to reserve the use of `forget` so that
//! searching the code base for it will still just show up actual
//! potential memory leaks as well as potential improper bypasses of
//! the linearity: `forget` on an enclosing data structure bypasses
//! the check and can be done for any data type; whereas `bury()` is
//! only available to the code declaring the linear token type, hence
//! under the control of the library.

//! Sadly the compiler error messages when a linear instance is
//! dropped are not very helpful, the only information one can
//! currently (as of rustc 1.87.0) glean from them are the type names
//! (also, you need to run `cargo build`, `cargo check` does not
//! report the drop). If you cannot find the place where an instance
//! is dropped, then compile with the `DEBUG_LINEAR` env variable set
//! to `1` (or `y` or `t`) and run your program--instead of a compile
//! time failure the drop implementation is then generating a run time
//! panic, which, when run with `RUST_BACKTRACE=1`, will show you
//! where the drop happens. Sadly there are cases where you will see
//! no run time drops, while the compiler still complains, e.g. if you
//! store a linear type in an `Option` or `Vec`, which is better
//! avoided.

// Somehow cannot provide Debug implementation for even just the
// wrapper type $T, or more to the point, using println! or
// dbg!(&v). Why?

use std::{marker::PhantomData, mem};

struct Inner<T, IN>(PhantomData<(T, IN)>);

/// A type that cannot be dropped.
pub struct UndroppableWithin<T, IN>(mem::ManuallyDrop<Inner<T, IN>>);

impl<T, IN> UndroppableWithin<T, IN> {
    pub fn new() -> Self {
        Self(mem::ManuallyDrop::new(Inner(PhantomData)))
    }
}

impl<T, IN> Drop for UndroppableWithin<T, IN> {
    fn drop(&mut self) {
        #[cfg(feature = "debug_linear")]
        panic!(
            "attempt to Drop an instance of the linear type {} \
             contained inside the type {}. \
             Instances of the latter type need to be passed to a cleanup function, \
             which itself must call `bury()` on the instance of the first type.",
            std::any::type_name::<T>(),
            std::any::type_name::<IN>(),
        );
        #[cfg(not(feature = "debug_linear"))]
        const {
            panic!(
                "attempt to Drop an instance of a linear type. \
                 Instances of the types mentioned as the type parameters to \
                 `UndroppableWithin` must be passed to cleanup functions: \
                 for users of the second type, the function(s) provided by that \
                 type are relevant; the implementations of those functions \
                 need to pass the instance of the first type to `bury()`."
            );
        }
    }
}

/// Create a new token type `$T` that is linear, i.e. whose instances
/// are undroppable, meant to be embedded in the type `$FOR`. `$FOR`
/// is reported as the type that is being dropped, when a `$T` is
/// dropped! Call `$T::new()` to create an instance, and call `bury()`
/// to get rid of it, preferably inside a method of `$FOR` that is
/// consuming the `$FOR` object and cleans it up.
#[macro_export]
macro_rules! def_linear {
    { $T:tt in $FOR:ty } => {
        struct $T($crate::linear::UndroppableWithin<$T, $FOR>);

        impl $T {
            fn new() -> Self {
                Self($crate::linear::UndroppableWithin::new())
            }

            pub fn bury(self) {
                std::mem::forget(self.0)
            }
        }
    }
}

#[test]
fn t_size() {
    assert_eq!(std::mem::size_of::<UndroppableWithin<u32, bool>>(), 0);
    struct Bar(Foo);
    def_linear!(Foo in Bar);
    assert_eq!(std::mem::size_of::<Foo>(), 0);
    assert_eq!(std::mem::size_of::<Bar>(), 0);
    let x = Foo::new();
    x.bury();
}
