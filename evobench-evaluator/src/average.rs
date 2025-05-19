//! Can't find in std lib?

pub trait Average {
    type Result;
    /// Rounding up for integer results where the result is in the
    /// middle between values.
    fn average(self) -> Self::Result;
}

impl Average for (u64, u64) {
    type Result = u64;
    fn average(self) -> Self::Result {
        let (a, b) = self;
        let sum = a as u128 + b as u128;
        ((sum + 1) / 2) as u64
    }
}

#[test]
fn t_average() {
    assert_eq!((20, 10).average(), 15);
    assert_eq!((10, 11).average(), 11);
}
