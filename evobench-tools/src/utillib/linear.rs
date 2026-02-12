//! Run-time "linear types"--warning in the `Drop` implementation in
//! release builds, panic in debug builds. Attempts at using the idea
//! of panicking in const does not work in practice (it appears that
//! drop templates are instantiated before being optimized away, hence
//! e.g. returning them in `Result::Ok` is not possible since it
//! apparently unconditionally instantiates the drop for Result which
//! instantiates the drop for the Ok value even if never used, but I
//! did not manage to analyze the binary to detect drop use after the
//! optimizer either).

//! Only token types are supported: embedding such a token type inside
//! a larger data structure makes the larger data structure run-time
//! checked for linearity, too. That type then has to provide a moving
//! method or function that throws away the linear token by calling
//! `bury()` on it. With that approach, no wrapper around the
//! containing data structure is needed, which might be cleaner?

//! Original idea and partially code came from
//! https://jack.wrenn.fyi/blog/undroppable/ and
//! https://geo-ant.github.io/blog/2024/rust-linear-types-use-once/,
//! but again, doesn't appear to work in practice. There are also some
//! other crates going the runtime route, maybe the most-used one
//! being <https://crates.io/crates/drop_bomb>.

//! Calling `std::mem::leak` (even on a containing data structure)
//! by-passes the linearity.  It is recommended to only ever use the
//! `bury()` method to get rid of a linear token, and reserve the use
//! of `std::mem::forget` for other purposes, so that searching the
//! code base for it will still just show up actual potential memory
//! leaks as well as potential improper bypasses of the linearity:
//! `forget` on an enclosing data structure bypasses the check and can
//! be done for any data type; whereas `bury()` is only available to
//! the code declaring the linear token type, hence under the control
//! of the library.

use std::marker::PhantomData;

/// A type that cannot be dropped.
#[must_use]
pub struct UndroppableWithin<T, IN> {
    fatal: bool,
    inner: PhantomData<(T, IN)>,
}

impl<T, IN> UndroppableWithin<T, IN> {
    pub fn new(fatal: bool) -> Self {
        let inner = PhantomData;
        Self { fatal, inner }
    }
}

impl<T, IN> Drop for UndroppableWithin<T, IN> {
    fn drop(&mut self) {
        let is_debug;
        #[cfg(not(debug_assertions))]
        {
            use crate::utillib::bool_env::get_env_bool;

            is_debug = get_env_bool("DEBUG_LINEAR", Some(false))
                .expect("no invalid DEBUG_LINEAR variable");
        }
        #[cfg(debug_assertions)]
        {
            is_debug = true;
        }
        if is_debug || self.fatal {
            panic!(
                "attempt to Drop an instance of the linear type {} \
                 contained inside the type {}. \
                 Instances of the latter type need to be passed to a cleanup function, \
                 which itself must call `bury()` on the instance of the first type.",
                std::any::type_name::<T>(),
                std::any::type_name::<IN>(),
            );
        } else {
            crate::warn!(
                "WARNING: attempt to Drop an instance of the linear type {} \
                 contained inside the type {}. \
                 Instances of the latter type need to be passed to a cleanup function, \
                 which itself must call `bury()` on the instance of the first type.",
                std::any::type_name::<T>(),
                std::any::type_name::<IN>(),
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
        #[must_use]
        struct $T($crate::utillib::linear::UndroppableWithin<$T, $FOR>);

        impl $T {
            fn new(fatal: bool) -> Self {
                Self($crate::utillib::linear::UndroppableWithin::new(fatal))
            }

            pub fn bury(self) {
                std::mem::forget(self.0)
            }
        }
    }
}

#[test]
fn t_size() {
    assert_eq!(std::mem::size_of::<UndroppableWithin<u32, bool>>(), 1);
    #[allow(unused)]
    struct Bar(Foo);
    def_linear!(Foo in Bar);
    assert_eq!(std::mem::size_of::<Foo>(), 1);
    assert_eq!(std::mem::size_of::<Bar>(), 1);
    let x = Foo::new(true);
    x.bury();
}
