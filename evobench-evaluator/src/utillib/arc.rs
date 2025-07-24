use std::sync::Arc;

pub fn arc<T>(v: &Arc<T>) -> Arc<T> {
    v.clone()
}

pub trait CloneArc {
    fn clone_arc(&self) -> Self;
}

impl<T: ?Sized> CloneArc for Arc<T> {
    fn clone_arc(&self) -> Self {
        self.clone()
    }
}
