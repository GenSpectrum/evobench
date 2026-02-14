//! Convert tuples of `Result` values into a `Result` of the `Ok`
//! values so that errors can be propagated easily. (There is also
//! `tuple-transpose` crate offering the same, but it might not stay
//! around and has no docs.)

pub trait TupleTranspose {
    type Output;
    fn transpose(self) -> Self::Output;
}

impl<V1, E> TupleTranspose for Result<V1, E> {
    type Output = Result<V1, E>;

    fn transpose(self) -> Self::Output {
        self
    }
}

impl<V1, V2, E> TupleTranspose for (Result<V1, E>, Result<V2, E>) {
    type Output = Result<(V1, V2), E>;

    fn transpose(self) -> Self::Output {
        Ok((self.0?, self.1?))
    }
}

impl<V1, V2, V3, E> TupleTranspose for (Result<V1, E>, Result<V2, E>, Result<V3, E>) {
    type Output = Result<(V1, V2, V3), E>;

    fn transpose(self) -> Self::Output {
        Ok((self.0?, self.1?, self.2?))
    }
}

impl<V1, V2, V3, V4, E> TupleTranspose
    for (Result<V1, E>, Result<V2, E>, Result<V3, E>, Result<V4, E>)
{
    type Output = Result<(V1, V2, V3, V4), E>;

    fn transpose(self) -> Self::Output {
        Ok((self.0?, self.1?, self.2?, self.3?))
    }
}

impl<V1, V2, V3, V4, V5, E> TupleTranspose
    for (
        Result<V1, E>,
        Result<V2, E>,
        Result<V3, E>,
        Result<V4, E>,
        Result<V5, E>,
    )
{
    type Output = Result<(V1, V2, V3, V4, V5), E>;

    fn transpose(self) -> Self::Output {
        Ok((self.0?, self.1?, self.2?, self.3?, self.4?))
    }
}
