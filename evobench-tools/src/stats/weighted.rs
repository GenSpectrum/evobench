//! Weighted values, e.g. for measurements taken with gaps, where the
//! same value is assumed to be valid for the gaps.

use std::{
    collections::BTreeMap,
    num::NonZeroU32,
    ops::Bound::{Included, Unbounded},
    ops::Index,
};

// Get the entry at key, or the next-lower one
fn lookup<'m, K: Ord, V>(map: &'m BTreeMap<K, V>, key: &K) -> (&'m K, &'m V) {
    map.range((Unbounded, Included(key)))
        .next_back()
        .expect("we fill in index 0 thus will always find a value")
}

pub const WEIGHT_ONE: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(1) };

/// Representation of statistical probes: their value with the count
/// of skipped runs that need to be compensated for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WeightedValue {
    // Keep the order of fields unchanged, it matters for the sorting
    // order!
    /// The original measured value
    pub value: u64,
    /// How many times this should be counted
    pub weight: NonZeroU32,
}

/// As is, only guarantees to allow up to u32 inputs, as it uses u64
/// as the index internally!
#[derive(Debug, Clone)]
pub struct IndexedNumbers {
    index_to_value: BTreeMap<u64, u64>,
    /// Index to position after the "end" of the last value
    virtual_len: u64,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("got too many or too heavily weighted values")]
pub struct TooMuchIndexedNumbersWeightError;

impl IndexedNumbers {
    #[inline]
    pub fn virtual_len(&self) -> u64 {
        self.virtual_len
    }

    pub fn first(&self) -> Option<&u64> {
        // self.index_to_value.get(&0)
        self.index_to_value.first_key_value().map(|(_k, v)| v)
    }

    pub fn last(&self) -> Option<&u64> {
        // self.index_to_value.get(&self.virtual_len)
        self.index_to_value.last_key_value().map(|(_k, v)| v)
    }

    /// As is, only allows up to u32 inputs for sure, as it uses u64
    /// as the index internally, and the weights are u32, too; returns
    /// an error otherwise. Sorts the vec!
    pub fn from_unsorted_weighted_value_vec(
        weighted_values: &mut Vec<WeightedValue>,
    ) -> Result<Self, TooMuchIndexedNumbersWeightError> {
        weighted_values.sort();

        let mut index_to_value = BTreeMap::new();
        let mut virtual_len: u64 = 0;
        for WeightedValue { value, weight } in weighted_values {
            index_to_value.insert(virtual_len, *value);
            let weight = u64::from(u32::from(*weight));
            virtual_len = virtual_len
                .checked_add(weight)
                .ok_or(TooMuchIndexedNumbersWeightError)?;
        }
        Ok(IndexedNumbers {
            index_to_value,
            virtual_len,
        })
    }

    /// Only returns None if `index >= self.virtual_len` (or Self
    /// contains no entries, which is also covered by the former
    /// statement).
    pub fn get(&self, index: u64) -> Option<&u64> {
        if index < self.virtual_len {
            Some(lookup(&self.index_to_value, &index).1)
        } else {
            None
        }
    }
}

impl Index<u64> for IndexedNumbers {
    type Output = u64;

    fn index(&self, index: u64) -> &Self::Output {
        self.get(index).expect(
            "index must be smaller than the virtual_len, \
             and IndexedNumbers must not be empty",
        )
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn t_weight_one() {
        assert_eq!(WEIGHT_ONE, NonZeroU32::try_from(1).unwrap());
    }

    #[test]
    fn t_indexed_numbers() -> Result<()> {
        let mut nums = vec![
            WeightedValue {
                value: 10,
                weight: 1.try_into()?,
            },
            WeightedValue {
                value: 100,
                weight: 5.try_into()?,
            },
            WeightedValue {
                value: 4,
                weight: 2.try_into()?,
            },
            WeightedValue {
                value: 105,
                weight: 1.try_into()?,
            },
            WeightedValue {
                value: 3,
                weight: 2.try_into()?,
            },
        ];
        let indexed_nums = IndexedNumbers::from_unsorted_weighted_value_vec(&mut nums)?;
        dbg!((&nums, &indexed_nums));

        assert_eq!(indexed_nums[0], 3);
        assert_eq!(indexed_nums.first(), Some(&3));
        assert_eq!(indexed_nums[10], 105);
        assert_eq!(indexed_nums.last(), Some(&105));
        assert_eq!(indexed_nums[2], 4);
        assert_eq!(indexed_nums[3], 4);
        assert_eq!(indexed_nums[4], 10);

        assert_eq!(indexed_nums.get(10), Some(&105));
        assert_eq!(indexed_nums.get(11), None);

        Ok(())
    }
}
