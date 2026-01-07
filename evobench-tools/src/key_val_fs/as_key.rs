use std::borrow::Cow;

/// A type that can be used as the key (i.e. converted to a file name)
/// in a `KeyVal` database.
pub trait AsKey: Sized {
    /// Types implementing this trait can't have failing conversions!
    /// If you want to use a key type with a fallible conversion, it
    /// must have a conversion to a custom type that implements AsKey,
    /// and *that* conversion will need to be fallible. The result
    /// must never start with a `.` (leading dot is used for temporary
    /// files), or contain the `/` or `\0` characters, must be at
    /// least 1 and at most 254 bytes long, and *should* never contain
    /// control characters (this could make it a pain for people to
    /// use command line tools to work with the databases).
    fn as_filename_str(&self) -> Cow<'_, str>;

    /// Calls `as_filename_str` and asserts that the result complies
    /// to the rules mentioned above, panics if it does not.
    fn verified_as_filename_str(&self) -> Cow<'_, str> {
        let s = self.as_filename_str();
        let bytes = s.as_bytes();
        assert!(bytes.len() <= 254);
        assert!(bytes.len() >= 1);
        assert!(bytes[0] != b'.');
        assert!(!bytes.contains(&b'/'));
        assert!(!bytes.contains(&b'\0'));
        s
    }

    /// Must convert the output of `as_filename_str` back into a Self
    /// equal to the original self. If not possible (e.g. a human
    /// placed an invalid file), return None.
    fn try_from_filename_str(file_name: &str) -> Option<Self>;
}
