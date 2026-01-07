//! Cryptographically hash a data structure

// `bincode` has its own introspection macro. And there's not even a
// widely used serde+bincode crate? So, go with serde_json.

use base64::Engine;
use serde::Serialize;
use sha2::Digest;

/// Calculate a base64-encoded (in URL-safe variant, i.e. no '/' or
/// '+') hash value
pub fn crypto_hash<T: Serialize>(val: &T) -> String {
    // When do serializers to string ever give errors?
    let s = serde_json::to_string(val).expect("can't fail we hope?");
    let mut hasher = sha2::Sha256::new();
    hasher.update(&s);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&hasher.finalize())
}
