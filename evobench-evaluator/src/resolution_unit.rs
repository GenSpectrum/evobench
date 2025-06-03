/// Attach a physical unit (or "count" or similar) to a type.
pub trait ResolutionUnit: Into<u64> {
    /// The physical unit for 1 increment of the resolution of the
    /// type (i.e. the unit for the result of the conversion into u64)
    const RESOLUTION_UNIT_SHORT: &str;
}

impl ResolutionUnit for u64 {
    const RESOLUTION_UNIT_SHORT: &str = "occurrences";
}
