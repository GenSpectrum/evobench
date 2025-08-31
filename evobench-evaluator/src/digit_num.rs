//! Numbers based on decimal digits, for correctness and
//! introspection, not performance. Useful for writing some kinds of
//! tests.

use std::{fmt::Display, io::Write};

use rand::Rng;

// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Digit(u8);

impl Display for Digit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Write::write_char(f, (self.0 + b'0') as char)
    }
}

#[derive(thiserror::Error, Debug)]
#[error("out of range")]
pub struct OutOfRange;

impl TryFrom<u8> for Digit {
    type Error = OutOfRange;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value < 10 {
            Ok(Self(value))
        } else {
            Err(OutOfRange)
        }
    }
}

impl Digit {
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();
        let d = rng.gen_range(0..10);
        d.try_into().expect("no bug")
    }
}

// -----------------------------------------------------------------------------

// `PRECISION` is the number of decimal digits after the dot.
#[derive(Debug, Clone)]
pub struct DigitNum<const PRECISION: usize>(Vec<Digit>);

pub struct DigitNumFormat {
    pub underscores: bool,
    /// Omit "." if no digits come after it
    pub omit_trailing_dot: bool,
}

impl<const PRECISION: usize> DigitNum<PRECISION> {
    pub fn new() -> Self {
        Self(vec![])
    }

    pub fn into_changed_dot_position<const NEW_PRECISION: usize>(self) -> DigitNum<NEW_PRECISION> {
        DigitNum(self.0)
    }

    pub fn push_lowest_digit(&mut self, d: Digit) {
        if d.0 == 0 && self.0.is_empty() {
            return;
        }
        self.0.push(d)
    }

    pub fn write<W: Write>(&self, params: DigitNumFormat, mut out: W) -> std::io::Result<()> {
        let DigitNumFormat {
            underscores,
            omit_trailing_dot,
        } = params;
        let len = self.0.len();
        let after_dot = PRECISION;
        let mut digits = self.0.iter();
        let len_missing_after_dot = if len > after_dot {
            let len_before_dot = len - after_dot;
            let mut position = len_before_dot % 3;
            if position == 0 {
                position = 3;
            }
            for _ in 0..len_before_dot {
                if position == 0 {
                    if underscores {
                        write!(out, "_")?;
                    }
                    position = 3;
                }
                position -= 1;
                let d = digits.next().expect("digits till lowest one are present");
                write!(out, "{d}")?;
            }
            0
        } else {
            write!(out, "0")?;
            after_dot - len
        };

        if after_dot == 0 && omit_trailing_dot {
            return Ok(());
        }

        write!(out, ".")?;

        let mut position = 3;

        for _ in 0..len_missing_after_dot {
            if position == 0 {
                if underscores {
                    write!(out, "_")?;
                }
                position = 3;
            }
            position -= 1;
            write!(out, "0")?;
        }
        for d in digits {
            if position == 0 {
                if underscores {
                    write!(out, "_")?;
                }
                position = 3;
            }
            position -= 1;
            write!(out, "{d}")?;
        }
        Ok(())
    }

    pub fn to_string_with_params(&self, params: DigitNumFormat) -> String {
        let mut s = Vec::new();
        self.write(params, &mut s)
            .expect("no errors writing to String");
        String::from_utf8(s).expect("no non-utf8")
    }
}

/// Note: converts to the lowest digit, ignores PRECISION!
impl<const PRECISION: usize> TryFrom<&DigitNum<PRECISION>> for u64 {
    type Error = OutOfRange;

    fn try_from(value: &DigitNum<PRECISION>) -> Result<Self, Self::Error> {
        let mut num: u64 = 0;
        for Digit(d) in value.0.iter() {
            num = num.checked_mul(10).ok_or(OutOfRange)?;
            num = num.checked_add(u64::from(*d)).ok_or(OutOfRange)?;
        }
        Ok(num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_digit_num() {
        assert!(Digit::try_from(10).is_err());
        let mut num: DigitNum<6> = DigitNum::new();
        num.push_lowest_digit(0.try_into().unwrap());
        num.push_lowest_digit(3.try_into().unwrap());
        num.push_lowest_digit(9.try_into().unwrap());
        assert_eq!(
            num.to_string_with_params(DigitNumFormat {
                underscores: false,
                omit_trailing_dot: false
            }),
            "0.000039"
        );
        assert_eq!(
            num.to_string_with_params(DigitNumFormat {
                underscores: true,
                omit_trailing_dot: false
            }),
            "0.000_039"
        );
        for d in [7, 1, 3, 4] {
            num.push_lowest_digit(d.try_into().unwrap());
        }
        assert_eq!(
            num.to_string_with_params(DigitNumFormat {
                underscores: false,
                omit_trailing_dot: false
            }),
            "0.397134"
        );
        for d in [5, 3, 5, 2, 9] {
            num.push_lowest_digit(d.try_into().unwrap());
        }
        assert_eq!(
            num.to_string_with_params(DigitNumFormat {
                underscores: false,
                omit_trailing_dot: false
            }),
            "39713.453529"
        );
        assert_eq!(
            num.to_string_with_params(DigitNumFormat {
                underscores: true,
                omit_trailing_dot: false
            }),
            "39_713.453_529"
        );
        let num_u64 = u64::try_from(&num).unwrap();
        assert_eq!(num_u64, 39713453529);
    }
}
