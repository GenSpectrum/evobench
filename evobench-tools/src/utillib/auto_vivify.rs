pub trait AutoVivify<T> {
    fn auto_get_mut<F>(&mut self, i: usize, gen_val: F) -> &mut T
    where
        F: FnMut() -> T;
}

impl<T> AutoVivify<T> for Vec<T> {
    #[inline]
    fn auto_get_mut<'s, F>(&'s mut self, i: usize, gen_val: F) -> &'s mut T
    where
        F: FnMut() -> T,
    {
        if let Some(rf) = self.get_mut(i) {
            // Need hack for this well-known borrow checker issue
            let ptr: *mut T = rf;
            let rf: &mut T = unsafe {
                // Safe because life time 's is valid for it.
                &mut *ptr
            };
            rf
        } else {
            self.resize_with(i + 1, gen_val);
            &mut self[i]
        }
    }
}
