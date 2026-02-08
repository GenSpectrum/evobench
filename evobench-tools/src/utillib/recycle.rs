use std::collections::BTreeSet;

pub trait RecycleVec {
    /// Recycle the storage of `self` by returning a new Vec that
    /// reuses it.
    fn recycle_vec<U>(self) -> Vec<U>;
}

impl<T> RecycleVec for Vec<T> {
    #[inline]
    fn recycle_vec<U>(mut self) -> Vec<U> {
        self.clear();
        self.into_iter().map(|_| unreachable!()).collect()
    }
}

// Does this work, too?
impl<T> RecycleVec for BTreeSet<T> {
    #[inline]
    fn recycle_vec<U>(mut self) -> Vec<U> {
        self.clear();
        self.into_iter().map(|_| unreachable!()).collect()
    }
}
