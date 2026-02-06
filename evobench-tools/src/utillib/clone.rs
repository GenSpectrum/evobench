#[macro_export]
macro_rules! clone {
    { $var:ident } => {
        let $var = $var.clone();
    }
}
