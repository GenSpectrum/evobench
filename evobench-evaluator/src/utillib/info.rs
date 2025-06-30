#[macro_export]
macro_rules! info_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprintln!($($arg)*);
        }
    }
}

#[macro_export]
macro_rules! info_noln_if {
    { $verbose:expr, $($arg:tt)* } => {
        if $verbose {
            eprint!($($arg)*);
        }
    }
}
