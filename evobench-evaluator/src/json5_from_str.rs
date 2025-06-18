//! The latest json5, 0.4.1, includes location info with errors, but
//! its `Display` implementation does not show the location; also it
//! does not offer a method to retrieve the location, only pattern
//! matching which they indicate will break with future versions.

use std::fmt::Display;

use json5::Location;
use serde::Deserialize;

pub struct Json5FromStrLocation(pub Location);

impl Display for Json5FromStrLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Location { line, column } = &self.0;
        write!(f, "{line}:{column}")
    }
}

pub fn json5_error_location(e: &json5::Error) -> Option<Json5FromStrLocation> {
    match e {
        json5::Error::Message { msg: _, location } => location
            .as_ref()
            .map(|location| Json5FromStrLocation(location.clone())),
    }
}

#[derive(Debug, thiserror::Error)]
pub struct Json5FromStrError(pub json5::Error);

impl Json5FromStrError {
    pub fn message_without_location(&self) -> &str {
        match &self.0 {
            json5::Error::Message { msg, location: _ } => msg,
        }
    }

    pub fn location(&self) -> Option<Json5FromStrLocation> {
        json5_error_location(&self.0)
    }
}

impl Display for Json5FromStrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = self.message_without_location();
        if let Some(location) = self.location() {
            write!(f, "{msg} at line:column {location}")
        } else {
            write!(f, "{msg}")
        }
    }
}

pub fn json5_from_str<'t, T: Deserialize<'t>>(s: &'t str) -> Result<T, Json5FromStrError> {
    json5::from_str(s).map_err(Json5FromStrError)
}
