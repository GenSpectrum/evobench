/// A shorter way to create a `anyhow::Result` with error context information
///
/// Instead of `.with_context(|| anyhow!("while doing {}", 1 + 1))`,
/// this allows writing `.map_err(ctx!("while doing {}", 1 + 1))`.
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
