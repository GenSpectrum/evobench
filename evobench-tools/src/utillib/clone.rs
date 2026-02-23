/// Shadow a variable with a clone of itself (in preparation for making closures)
#[macro_export]
macro_rules! clone {
    { $var:ident } => {
        let $var = $var.clone();
    }
}
