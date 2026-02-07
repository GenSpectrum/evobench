pub trait Invert {
    type Target;
    fn invert(self) -> Self::Target;
}

impl<A, B> Invert for Result<A, B> {
    type Target = Result<B, A>;

    fn invert(self) -> Result<B, A> {
        match self {
            Ok(v) => Err(v),
            Err(e) => Ok(e),
        }
    }
}
