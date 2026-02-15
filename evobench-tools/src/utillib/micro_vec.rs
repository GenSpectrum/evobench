/// Currently `size_of::<MicroVec<T>>()` is at least 2 machine
/// words. Could optimize via tagged-pointer-as-enum, tagged-pointer,
/// enum-ptr, tagged-box crates.

/// A vector that is optimized for storing 0 or 1 elements. >1
/// elements are stored in a normal Vec, slowly.
#[derive(Debug, PartialEq, Eq)]
pub enum MicroVec<T> {
    None,
    One(T),
    More(Box<Vec<T>>),
}

impl<T> Default for MicroVec<T> {
    #[inline]
    fn default() -> Self {
        MicroVec::new()
    }
}

impl<T, const N: usize> From<[T; N]> for MicroVec<T> {
    fn from(values: [T; N]) -> Self {
        let mut vec = MicroVec::new();
        for v in values {
            vec.push(v);
        }
        vec
    }
}

impl<T: Clone> From<&[T]> for MicroVec<T> {
    fn from(values: &[T]) -> Self {
        let mut vec = MicroVec::new();
        for v in values {
            vec.push(v.clone());
        }
        vec
    }
}

impl<T> MicroVec<T> {
    #[inline]
    pub fn new() -> Self {
        Self::None
    }

    pub fn len(&self) -> usize {
        match self {
            MicroVec::None => 0,
            MicroVec::One(_) => 1,
            MicroVec::More(items) => items.len(),
        }
    }

    pub fn push(&mut self, val: T) {
        match self {
            MicroVec::None => {
                *self = MicroVec::One(val);
            }
            MicroVec::One(_) => {
                let mut removed = MicroVec::None;
                std::mem::swap(self, &mut removed);
                match removed {
                    MicroVec::One(v0) => {
                        *self = MicroVec::More(vec![v0, val].into());
                    }
                    _ => unreachable!(),
                }
            }
            MicroVec::More(items) => {
                items.push(val);
            }
        }
    }
}

#[macro_export]
macro_rules! microvec {
    [] => {
        $crate::utillib::micro_vec::MicroVec::None
    };
    [ $v:expr ] => {
        $crate::utillib::micro_vec::MicroVec::One($v)
    };
    [ $($e:tt)* ] => {
        {
            let mut vec = $crate::utillib::micro_vec::MicroVec::None;
            for v in [$($e)*] {
                vec.push(v);
            }
            vec
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_1() {
        let mut vec: MicroVec<u32> = Default::default();

        // A little disappointing but not caring about this now
        assert_eq!(size_of_val(&vec), 16);

        assert_eq!(vec.len(), 0);
        assert_eq!(&microvec![], &vec);

        vec.push(123);
        assert_eq!(vec.len(), 1);
        assert_eq!(&microvec![123], &vec);

        vec.push(124);
        assert_eq!(vec.len(), 2);

        assert_eq!(&microvec![123, 124], &vec);
    }
}
