use std::ops::{Add, Sub};

use num_traits::CheckedSub;
use serde::{Deserialize, Serialize};

macro_rules! define_time {
    { $_Time:ident, $_sec:tt, $max__sec:tt } => {

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $_Time {
            pub sec: u32,
            pub $_sec: u32,
        }

        impl $_Time {
            /// Panics if sub-second part is not within range. XX how better
            /// with serde?
            pub fn check(self) {
                assert!(self.$_sec < $max__sec)
            }

            pub fn is_valid(self)-> bool {
                self.$_sec < $max__sec
            }

            pub fn new(sec: u32, $_sec: u32) -> Option<Self> {
                if $_sec < $max__sec {
                    Some(Self{ sec, $_sec})
                }  else {
                    None
                }
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
    pub fn to_nsec(self) -> u64 {
        self.sec as u64 * 1_000_000_000 + (self.usec as u64 * 1000)
    }
}

// `struct timespec` in POSIX.

define_time!(NanoTime, nsec, 1_000_000_000);

impl NanoTime {
    pub fn to_nsec(self) -> u64 {
        self.sec as u64 * 1_000_000_000 + self.nsec as u64
    }
    pub fn from_nsec(nseconds: u64) -> Option<Self> {
        let sec = nseconds / 1_000_000_000;
        let nsec = nseconds % 1_000_000_000;
        Some(Self {
            sec: sec.try_into().ok()?,
            nsec: nsec.try_into().expect("always in range"),
        })
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
