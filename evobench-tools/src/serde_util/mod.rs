use std::{cell::RefCell, fmt::Display, path::Path};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ctx;

/// Read the file at `path` to RAM, then deserialize it using
/// `serde_json`. If you want to support json5 or ron, use
/// `config_file.rs` instead.
pub fn serde_read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    (|| -> Result<_> {
        let s = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&s)?)
    })()
    .map_err(ctx!("reading file {path:?}"))
}

pub struct CanonicalJson<'t>(pub &'t Value);

fn display_canonical_json(json: &Value, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match json {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            write!(f, "{}", json)
        }
        Value::Object(map) => {
            let display_keyval =
                |(key, val, _): &(&String, &Value, _), f: &mut std::fmt::Formatter<'_>| {
                    // XX oh, clone the string for no good reason;
                    // another way to get printing?
                    write!(f, "{}:", Value::String((*key).to_owned()))?;
                    display_canonical_json(val, f)
                };

            f.write_str("{")?;
            let mut keyvals: Vec<(&String, &Value, RefCell<Option<String>>)> = map
                .iter()
                .map(|(key, val)| (key, val, RefCell::new(None)))
                .collect();
            if !keyvals.is_empty() {
                keyvals.sort_by(|a, b| {
                    a.0.cmp(b.0).then_with(|| {
                        let mut ar = a.2.borrow_mut();
                        if ar.is_none() {
                            *ar = Some(CanonicalJson(&a.1).to_string());
                        }
                        let mut br = b.2.borrow_mut();
                        if br.is_none() {
                            *br = Some(CanonicalJson(&b.1).to_string());
                        }
                        ar.as_ref()
                            .expect("just set")
                            .cmp(br.as_ref().expect("just_set"))
                    })
                });
                for keyval in &keyvals[0..keyvals.len() - 1] {
                    display_keyval(keyval, f)?;
                    f.write_str(",")?;
                }
                display_keyval(keyvals.last().expect("checked non-empty"), f)?;
            }
            f.write_str("}")
        }
    }
}

impl<'t> Display for CanonicalJson<'t> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        display_canonical_json(&self.0, f)
    }
}
