//! Avoid errors when implementing fallbacks with option structs.

pub trait FallingBackTo {
    /// Use values from self if present, otherwise use those from
    /// `fallback`.
    fn falling_back_to(self, fallback: &Self) -> Self;
}

#[macro_export]
macro_rules! fallback_to_option {
    { $fallback:ident . $field:ident } => {
        let $field = $field.or_else(|| $fallback.$field.clone());
    }
}

#[macro_export]
macro_rules! fallback_to_trait {
    { $fallback:ident . $field:ident } => {
        let $field = $field.falling_back_to(&$fallback.$field);
    }
}

#[macro_export]
macro_rules! fallback_to_default {
    { $default:ident . $field:ident } => {
        let $field = $field.unwrap_or($default.$field);
    }
}
