use std::{ffi::OsStr, num::NonZeroU32, str::FromStr};

use anyhow::{anyhow, bail, Result};
use kstring::KString;

use crate::serde::{proper_dirname::ProperDirname, proper_filename::ProperFilename};

/// The value type of a custom parameter--those values are passed as
/// environment variables and hence as strings, but they are parsed
/// when read from the user (config) to ensure correctness.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CustomParameterType {
    String,
    Filename,
    Dirname,
    Bool,
    NonZeroU32,
    U32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedCustomParameter {
    pub required: bool,
    pub r#type: CustomParameterType,
}

/// A checked value
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomParameterValue {
    r#type: CustomParameterType,
    value: KString,
}

impl CustomParameterValue {
    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn checked_from(r#type: CustomParameterType, value: &KString) -> Result<Self> {
        match r#type {
            CustomParameterType::String => (),
            CustomParameterType::Filename => {
                let _ = ProperFilename::from_str(value).map_err(|e| anyhow!("expecting {e}"))?;
            }
            CustomParameterType::Dirname => {
                let _ = ProperDirname::from_str(value).map_err(|e| anyhow!("expecting {e}"))?;
            }
            CustomParameterType::Bool => match value.as_str() {
                "0" | "1" => (),
                _ => bail!(
                    "string not valid as a boolean custom parameter, \
                     expecting \"0\" or \"1\": {value:?}"
                ),
            },
            CustomParameterType::NonZeroU32 => {
                let _ = NonZeroU32::from_str(value)?;
            }
            CustomParameterType::U32 => {
                let _ = u32::from_str(value)?;
            }
        }
        Ok(Self {
            r#type,
            value: value.clone(),
        })
    }
}

impl AsRef<OsStr> for CustomParameterValue {
    fn as_ref(&self) -> &OsStr {
        self.as_str().as_ref()
    }
}
