use std::any::type_name;

pub fn short_name(s: &str) -> &str {
    let mut prev_i = None;
    for (i, c) in s.char_indices().rev() {
        if c == ':' {
            break;
        }
        prev_i = Some(i);
    }
    // Fall back to the whole thing if the name at the end is 0
    // characters long--is this even possible even via quotation?
    &s[prev_i.unwrap_or(0)..]
}

/// `type_name` without the namespace
pub fn type_name_short<T: ?Sized>() -> &'static str {
    short_name(type_name::<T>())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Foo;

    #[test]
    fn t_() {
        assert_eq!(type_name_short::<Foo>(), "Foo");
    }
}
