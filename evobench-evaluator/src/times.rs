//! Time durations in microseconds and nanoseconds, plus conversions
//! between them as well as traits for a common `u64` based
//! representation and getting the unit as human-readable string from
//! the type for doing type safe statistics (works with the `stats`
//! module). Also includes formatting as strings in milliseconds but
//! padded to the precision.

use std::fmt::{Debug, Display};
use std::ops::{Add, Sub};

use num_traits::CheckedSub;
use serde::{Deserialize, Serialize};

use crate::resolution_unit::ResolutionUnit;

pub trait ToStringMilliseconds {
    /// "1234.567890" or as many digits as the type has precision for
    /// (always that many, filling with zeroes)
    fn to_string_ms(&self) -> String;
}

pub trait ToStringSeconds {
    /// "1.234567890" or as many digits as the type has precision for
    /// (always that many, filling with zeroes)
    fn to_string_seconds(&self) -> String;
}

pub trait FromMicroseconds: Sized {
    fn from_usec(useconds: u64) -> Option<Self>;
}

pub trait ToNanoseconds {
    fn to_nsec(self) -> u64;
}

/// To nsec or usec depending on the type
pub trait ToIncrements {
    fn to_increments(self) -> u64;
}

/// (Only used in tests, should not be treated as important, or
/// ToStringMilliseconds removed?)
pub trait Time:
    ToStringMilliseconds
    + FromMicroseconds
    + From<u64>
    + Display
    + ToNanoseconds
    + Debug
    + Copy
    + ToIncrements
{
}

impl Time for MicroTime {}
impl ResolutionUnit for MicroTime {
    const RESOLUTION_UNIT_SHORT: &str = "us";
}
impl Time for NanoTime {}
impl ResolutionUnit for NanoTime {
    const RESOLUTION_UNIT_SHORT: &str = "ns";
}

fn print_milli_micro(f: &mut std::fmt::Formatter<'_>, milli: u32, micro: u32) -> std::fmt::Result {
    write!(f, "{milli}.{micro:03} ms")
}

fn print_milli_micro_nano(
    f: &mut std::fmt::Formatter<'_>,
    milli: u32,
    micro: u32,
    nano: u32,
) -> std::fmt::Result {
    write!(f, "{milli}.{micro:03}_{nano:03} ms")
}

fn print_micro_nano(f: &mut std::fmt::Formatter<'_>, micro: u32, nano: u32) -> std::fmt::Result {
    write!(f, "{micro}.{nano:03} us")
}

macro_rules! define_time {
    { $_Time:ident, $_sec:tt, $max__sec:tt } => {

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $_Time {
            sec: u32,
            $_sec: u32,
        }

        impl $_Time {
            /// Panics if sub-second part is not within range. XX how better
            /// with serde?
            pub fn check(self) {
                assert!(self.is_valid())
            }

            pub fn is_valid(self)-> bool {
                self.$_sec < $max__sec
            }

            pub fn new(sec: u32, $_sec: u32) -> Option<Self> {
                let slf = Self { sec, $_sec };
                if slf.is_valid() {
                    Some(slf)
                }  else {
                    None
                }
            }

            pub fn sec(self) -> u32 { self.sec }
            pub fn $_sec(self) -> u32 { self.$_sec }
        }

        impl ToIncrements for $_Time {
            fn to_increments(self) -> u64 {
                u64::from(self.sec) * $max__sec + u64::from(self.$_sec)
            }
        }

        impl Add for $_Time {
            type Output = $_Time;

            fn add(self, rhs: Self) -> Self::Output {
                const CUTOFF: u32 = $max__sec;
                let $_sec = self.$_sec + rhs.$_sec;
                if $_sec >= CUTOFF {
                    Self {
                        sec: self.sec + rhs.sec + 1,
                        $_sec: $_sec - CUTOFF,
                    }
                } else {
                    Self {
                        sec: self.sec + rhs.sec,
                        $_sec,
                    }
                }
            }
        }

        impl Sub for $_Time {
            type Output = $_Time;

            fn sub(self, rhs: Self) -> Self::Output {
                self.checked_sub(&rhs).expect("number overflow")
            }
        }

        impl CheckedSub for $_Time {
            fn checked_sub(&self, rhs: &Self) -> Option<Self> {
                let sec = self.sec.checked_sub(rhs.sec)?;
                match self.$_sec.checked_sub(rhs.$_sec) {
                    Some($_sec) => Some(Self { sec, $_sec }),
                    None => Some(Self {
                        sec: sec - 1,
                        $_sec: (self.$_sec + $max__sec) - rhs.$_sec,
                    }),
                }
            }
        }
    }
}

// `struct timeval` in POSIX.

define_time!(MicroTime, usec, 1_000_000);

impl MicroTime {
    fn to_usec(self) -> u64 {
        self.sec as u64 * 1_000_000 + (self.usec as u64)
    }
}

impl FromMicroseconds for MicroTime {
    fn from_usec(useconds: u64) -> Option<Self> {
        let sec = useconds / 1_000_000;
        let usec = useconds % 1_000_000;
        Some(Self {
            sec: sec.try_into().ok()?,
            usec: usec.try_into().expect("always in range"),
        })
    }
}

impl ToNanoseconds for MicroTime {
    fn to_nsec(self) -> u64 {
        self.sec as u64 * 1_000_000_000 + (self.usec as u64 * 1000)
    }
}

/// Assumes microseconds. Panics for values outside the representable
/// range!
impl From<u64> for MicroTime {
    fn from(value: u64) -> Self {
        Self::from_usec(value).expect("outside representable range")
    }
}

/// Into microseconds.
impl From<MicroTime> for u64 {
    fn from(value: MicroTime) -> Self {
        value.to_usec()
    }
}

fn milli_micro(usec: u32) -> (u32, u32) {
    (usec / 1000, usec % 1000)
}

fn format_integer_with_undercores(digits: &str) -> String {
    digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(std::str::from_utf8)
        .collect::<Result<Vec<&str>, _>>()
        .unwrap()
        .join("_")
}

impl Display for MicroTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { sec, usec } = *self;
        if sec >= 1 {
            let (milli, micro) = milli_micro(usec);
            let sec_str = format_integer_with_undercores(&sec.to_string());
            write!(f, "{sec_str}.{milli:03}_{micro:03} s")
        } else if usec >= 1_000 {
            let (milli, micro) = milli_micro(usec);
            print_milli_micro(f, milli, micro)
        } else {
            let usec_str = format_integer_with_undercores(&usec.to_string());
            write!(f, "{usec_str} us")
        }
    }
}

// `struct timespec` in POSIX.

define_time!(NanoTime, nsec, 1_000_000_000);

impl NanoTime {
    pub fn from_nsec(nseconds: u64) -> Option<Self> {
        let sec = nseconds / 1_000_000_000;
        let nsec = nseconds % 1_000_000_000;
        Some(Self {
            sec: sec.try_into().ok()?,
            nsec: nsec.try_into().expect("always in range"),
        })
    }
}

impl FromMicroseconds for NanoTime {
    fn from_usec(useconds: u64) -> Option<Self> {
        let nsec = useconds.checked_mul(1000)?;
        Self::from_nsec(nsec)
    }
}

impl ToNanoseconds for NanoTime {
    fn to_nsec(self) -> u64 {
        self.sec as u64 * 1_000_000_000 + self.nsec as u64
    }
}

impl From<MicroTime> for NanoTime {
    fn from(value: MicroTime) -> Self {
        NanoTime {
            sec: value.sec,
            nsec: value.usec * 1000,
        }
    }
}

/// Assumes nanoseconds. Panics for values outside the representable
/// range!
impl From<u64> for NanoTime {
    fn from(value: u64) -> Self {
        Self::from_nsec(value).expect("outside representable range")
    }
}

/// Into nanoseconds.
impl From<NanoTime> for u64 {
    fn from(value: NanoTime) -> Self {
        value.to_nsec()
    }
}

fn milli_micro_nano(nsec: u32) -> (u32, u32, u32) {
    let usec = nsec / 1000;
    let nano = nsec % 1000;
    let (milli, micro) = milli_micro(usec);
    (milli, micro, nano)
}

impl Display for NanoTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { sec, nsec } = *self;
        if sec >= 1 {
            let (milli, micro, nano) = milli_micro_nano(nsec);
            let sec_str = format_integer_with_undercores(&sec.to_string());
            write!(f, "{sec_str}.{milli:03}_{micro:03}_{nano:03} s")
        } else {
            let (milli, micro, nano) = milli_micro_nano(nsec);
            if milli > 0 {
                print_milli_micro_nano(f, milli, micro, nano)
            } else if micro > 0 {
                print_micro_nano(f, micro, nano)
            } else {
                let nsec_str = format_integer_with_undercores(&nsec.to_string());
                write!(f, "{nsec_str} ns")
            }
        }
    }
}

impl ToStringMilliseconds for MicroTime {
    fn to_string_ms(&self) -> String {
        let ms = self.sec * 1000 + self.usec / 1_000;
        let usec_rest = self.usec % 1_000;
        format!("{ms}.{usec_rest:03}")
    }
}

impl ToStringMilliseconds for NanoTime {
    fn to_string_ms(&self) -> String {
        let ms = self.sec * 1000 + self.nsec / 1_000_000;
        let nsec_rest = self.nsec % 1_000_000;
        format!("{ms}.{nsec_rest:06}")
    }
}

impl ToStringSeconds for MicroTime {
    fn to_string_seconds(&self) -> String {
        let Self { sec, usec } = self;
        format!("{sec}.{usec:06}")
    }
}

impl ToStringSeconds for NanoTime {
    fn to_string_seconds(&self) -> String {
        let Self { sec, nsec } = self;
        format!("{sec}.{nsec:09}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digit_num::{Digit, DigitNum, DigitNumFormat};
    use rand::Rng;

    #[test]
    fn t_micro_time() {
        let a = MicroTime::new(2, 999_999).unwrap();
        let b = MicroTime::new(3, 1).unwrap();
        assert_eq!(a + b, MicroTime::new(6, 0).unwrap());
        assert_eq!(
            a + MicroTime::new(0, 2).unwrap(),
            MicroTime::new(3, 1).unwrap()
        );
        let t = |sec, usec| MicroTime::new(sec, usec).unwrap();
        assert_eq!(t(10, 2) - t(10, 1), t(0, 1));
        assert_eq!(t(10, 2) - t(10, 2), t(0, 0));
        // t(10, 2) - t(10, 3),
        assert_eq!(t(11, 2) - t(10, 2), t(1, 0));
        assert_eq!(t(11, 2) - t(10, 1), t(1, 1));
        assert_eq!(t(11, 2) - t(10, 3), t(0, 999_999));
        assert_eq!(b - a, MicroTime::new(0, 2).unwrap());
        assert_eq!(t(0, 999_999) + t(3, 999_999), t(4, 999_998));
        assert_eq!(t(4, 999_998) - t(3, 999_999), t(0, 999_999));
        assert_eq!(t(4, 999_998) - t(0, 999_999), t(3, 999_999));
    }

    #[test]
    #[should_panic]
    fn t_micro_time_panic() {
        let a = MicroTime::new(2, 999_999).unwrap();
        let b = MicroTime::new(3, 1).unwrap();
        let _ = a - b;
    }

    #[test]
    #[should_panic]
    fn t_micro_time_panic_new() {
        let _ = MicroTime::new(2, 1_000_000).unwrap();
    }

    #[test]
    fn t_nano_time() {
        let t = |sec, nsec| NanoTime::new(sec, nsec).unwrap();
        assert_eq!(t(4, 999_998) - t(0, 999_999), t(3, 999_999_999));
        assert_eq!(t(4, 999_999) - t(0, 999_998), t(4, 1));

        assert_eq!(t(0, 999_999_999) + t(3, 999_999_999), t(4, 999_999_998));
        assert_eq!(t(4, 999_999_998) - t(3, 999_999_999), t(0, 999_999_999));
        assert_eq!(t(4, 999_999_998) - t(0, 999_999_999), t(3, 999_999_999));
    }

    #[test]
    fn t_nano_time_convert() {
        let n = |sec, nsec| NanoTime::new(sec, nsec).unwrap();
        let u = |sec, usec| MicroTime::new(sec, usec).unwrap();
        assert_eq!(
            {
                let x: NanoTime = u(3, 490_000).into();
                x
            },
            n(3, 490_000_000),
        );
        assert_eq!(u(8, 30).to_nsec(), n(8, 30_000).to_nsec(),);

        assert_eq!(
            NanoTime::from_nsec(u(8, 30).to_nsec()).unwrap(),
            n(8, 30_000)
        );
    }

    fn test_stringification<
        const DIGITS_BELOW_MS: usize,
        const DIGITS_BELOW_S: usize,
        Time: ToStringMilliseconds
            + ToStringSeconds
            + FromMicroseconds
            + From<u64>
            + Display
            + ToNanoseconds
            + Debug
            + Copy
            + ToIncrements,
    >() {
        let mut num: DigitNum<DIGITS_BELOW_MS> = DigitNum::new();
        let mut num_seconds: DigitNum<DIGITS_BELOW_S> = DigitNum::new();
        let digits_above_ms = 10; // 10 is the max possible for just creating nums
        let num_digits_to_test = DIGITS_BELOW_MS + digits_above_ms;
        for _ in 0..num_digits_to_test {
            let num_u64: u64 = (&num).try_into().unwrap();
            let time = Time::from(num_u64);

            // Test `to_string_ms()`
            assert_eq!(
                time.to_string_ms(),
                num.to_string_with_params(DigitNumFormat {
                    underscores: false,
                    omit_trailing_dot: false
                })
            );

            // Test `to_string_seconds()`
            assert_eq!(
                time.to_string_seconds(),
                num_seconds.to_string_with_params(DigitNumFormat {
                    underscores: false,
                    omit_trailing_dot: false
                })
            );

            // Test `Display`
            let time_str = format!("{time}");
            let parts: Vec<_> = time_str.split(' ').collect();
            let (number, num_digits_below_seconds) = match parts.as_slice() {
                &[number, "ns"] => (number, 9),
                &[number, "us"] => (number, 6),
                &[number, "ms"] => (number, 3),
                &[number, "s"] => (number, 0),
                _ => unreachable!(),
            };
            let (expect_dot, expect_digits_after_dot) =
                if DIGITS_BELOW_S == num_digits_below_seconds {
                    (false, 0)
                } else {
                    (true, DIGITS_BELOW_S - num_digits_below_seconds)
                };
            let parts: Vec<&str> = number.split('.').collect();
            let digits = match parts.as_slice() {
                [left, right] => {
                    assert!(expect_dot);
                    // If given ns, and DIGITS_BELOW_S is 9, then
                    // right.len() is 0. Won't even have a dot.
                    let right_without_underscores = right.replace("_", "");
                    assert_eq!(right_without_underscores.len(), expect_digits_after_dot);
                    format!("{left}_{right}")
                }
                [left_only] => {
                    assert!(!expect_dot);
                    format!("{left_only}")
                }
                _ => unreachable!(),
            };
            let num_in_increments: DigitNum<0> = num.clone().into_changed_dot_position();
            assert_eq!(num_u64, u64::try_from(&num_in_increments).unwrap());
            assert_eq!(
                digits,
                num_in_increments.to_string_with_params(DigitNumFormat {
                    underscores: true,
                    omit_trailing_dot: true
                })
            );

            // Test `ToIncrements`
            assert_eq!(time.to_increments(), num_u64);

            // Test `ToNanoseconds`
            let ns = time.to_nsec();
            // num_u64 is in us or ns, depending on the type.
            let num_u64_multiplicator = match DIGITS_BELOW_S {
                6 => 1000,
                9 => 1,
                _ => unreachable!(),
            };
            assert_eq!(ns, num_u64 * num_u64_multiplicator);

            // Test `FromMicroseconds`
            let time2 = Time::from_usec(num_u64).expect("works for MicroTime::from, so also here");
            let ns2 = time2.to_nsec();
            assert_eq!(ns2, num_u64 * 1000);

            let digit = Digit::random();
            num.push_lowest_digit(digit);
            num_seconds.push_lowest_digit(digit);
        }
    }

    #[test]
    fn t_micro_time_stringification() {
        test_stringification::<3, 6, MicroTime>();
    }

    #[test]
    fn t_nano_time_stringification() {
        test_stringification::<6, 9, NanoTime>();
    }

    fn test_arithmetic<Time: From<u64> + Display + Debug + Copy + Add + Sub>()
    where
        <Time as Add>::Output: ToIncrements,
        <Time as Sub>::Output: ToIncrements,
    {
        let mut rng = rand::thread_rng();

        for _ in 0..100000 {
            // (* (- (expt 2 32) 1) 1000000 1/2)
            let max = 2147483647500000;
            let a: u64 = rng.gen_range(0..max);
            let ta = Time::from(a);
            let b: u64 = rng.gen_range(0..max);
            let tb = Time::from(b);
            let c = a + b;
            let tc = ta + tb;
            assert_eq!(c, tc.to_increments());
            let (d, td) = if a > b {
                (a - b, ta - tb)
            } else {
                (b - a, tb - ta)
            };
            assert_eq!(d, td.to_increments());
        }
    }

    #[test]
    fn t_micro_time_arithmetic() {
        test_arithmetic::<MicroTime>();
    }

    #[test]
    fn t_nano_time_arithmetic() {
        test_arithmetic::<NanoTime>();
    }
}
