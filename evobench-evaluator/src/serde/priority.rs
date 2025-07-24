// There's <https://crates.io/crates/ordered-float> but I haven't
// reviewed it, except I saw that it (in version 3) doesn't use
// TryFrom to construct floats, instead ordering NaN at the end. This
// doesn't seem right for priority use, thus making our own.

use std::{
    fmt::Display,
    ops::{Add, Neg},
    str::FromStr,
};

use serde::de::Visitor;

/// A priority level. The level is any orderable instance of a `f64`
/// value (i.e. not NAN).
#[derive(Debug, PartialEq, PartialOrd, Clone, Copy)]
pub struct Priority(f64);

impl Eq for Priority {}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other)
            .expect("always succeeds due to check in constructor")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("not a comparable number: {0}")]
pub struct NonComparableNumber(f64);

impl Priority {
    pub const HIGH: Priority = Priority::new_unchecked(1.);
    pub const NORMAL: Priority = Priority::new_unchecked(0.);
    pub const LOW: Priority = Priority::new_unchecked(-1.);

    /// This does not verify that `value` is comparable. Expect panics
    /// and other problems if it isn't! This function only exists for
    /// `const` purposes.
    pub const fn new_unchecked(value: f64) -> Self {
        Self(value)
    }

    pub fn new(value: f64) -> Result<Self, NonComparableNumber> {
        match value.partial_cmp(&1.23) {
            Some(_) => Ok(Self(value)),
            None => Err(NonComparableNumber(value)),
        }
    }

    pub fn add(self, difference: f64) -> Result<Self, NonComparableNumber> {
        Self::new(self.0 + difference)
    }

    pub fn sub(self, difference: f64) -> Result<Self, NonComparableNumber> {
        Self::new(self.0 - difference)
    }
}

impl Neg for Priority {
    type Output = Priority;

    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Add for Priority {
    type Output = Result<Priority, NonComparableNumber>;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.0 + rhs.0)
    }
}

impl Default for Priority {
    fn default() -> Self {
        Self::new_unchecked(0.)
    }
}

impl TryFrom<f64> for Priority {
    type Error = NonComparableNumber;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Priority> for f64 {
    fn from(value: Priority) -> Self {
        value.0
    }
}

impl TryFrom<f32> for Priority {
    type Error = NonComparableNumber;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value.into())
    }
}

impl FromStr for Priority {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "high" => Ok(Priority::HIGH),
            "normal" => Ok(Priority::NORMAL),
            "low" => Ok(Priority::LOW),
            _ => Ok(Priority::new(s.parse()?)?),
        }
    }
}

impl Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // XX round?
        self.0.fmt(f)
    }
}

struct OurVisitor;
impl<'de> Visitor<'de> for OurVisitor {
    type Value = Priority;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a floating point number")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Priority::new(v.parse().map_err(E::custom)?).map_err(E::custom)
    }
}

impl<'de> serde::Deserialize<'de> for Priority {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(OurVisitor)
    }
}

impl serde::Serialize for Priority {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
