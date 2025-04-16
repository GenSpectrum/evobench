use anyhow::{bail, Result};

use crate::log_message::Timing;

#[derive(Debug)]
pub struct Scope<'t> {
    // pub pn: &'t str, -- redundant since it's in Timing
    pub start: &'t Timing,
    pub end: &'t Timing,
}

impl<'t> Scope<'t> {
    pub fn new(start: &'t Timing, end: &'t Timing) -> Result<Self> {
        if start.pn == end.pn {
            Ok(Self { start, end })
        } else {
            bail!(
                "timings not from the same probe name: {:?} vs. {:?}",
                start.pn,
                end.pn
            )
        }
    }
    pub fn pn(&self) -> &'t str {
        &self.start.pn
    }
}
