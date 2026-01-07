#[macro_export]
macro_rules! ctx {
    ($fmt:tt) => {
        |e| anyhow::Context::context(Result::<(), _>::Err(e), format!($fmt))
            .err().unwrap()
    };
    ($fmt:tt, $($arg:tt)*) => {
        |e| anyhow::Context::context(Result::<(), _>::Err(e), format!($fmt, $($arg)*))
            .err().unwrap()
    };
}
