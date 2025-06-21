#[macro_export]
macro_rules! ctx {
    ($msg:expr) => {
        |e| anyhow::Context::context(Result::<(), _>::Err(e), $msg)
            .err().unwrap()
    };
    ($fmt:literal, $($arg:tt)*) => {
        |e| anyhow::Context::context(Result::<(), _>::Err(e), format!($fmt, $($arg)*))
            .err().unwrap()
    };
}
