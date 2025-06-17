//! A map that supports serde and construction via `Vec<KeyVal>`, and
//! that way, command line parsing via clap.

use std::collections::BTreeMap;

use super::key_val::KeyVal;

#[derive(Debug, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct Map<K: Ord, V>(BTreeMap<K, V>);

/// Relying on `KeyVal`, i.e. parsing a sequence of "FOO=bar" style
/// strings
impl<KV: AsRef<KeyVal>, T: IntoIterator<Item = KV>> From<T> for Map<String, String> {
    fn from(keyvals: T) -> Self {
        let mut m = BTreeMap::new();
        for kv in keyvals {
            let KeyVal { key, val } = kv.as_ref();
            m.insert(key.to_owned(), val.to_owned());
        }
        Self(m)
    }
}
