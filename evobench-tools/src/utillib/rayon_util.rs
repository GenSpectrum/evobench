pub trait ParRun {
    type Output;
    fn par_run(self) -> Self::Output;
}

impl<F1, F2, T1, T2> ParRun for (F1, F2)
where
    F1: FnOnce() -> T1 + Send,
    F2: FnOnce() -> T2 + Send,
    T1: Send,
    T2: Send,
{
    type Output = (T1, T2);

    fn par_run(self) -> Self::Output {
        let (f1, f2) = self;
        rayon::join(f1, f2)
    }
}

impl<F1, F2, F3, T1, T2, T3> ParRun for (F1, F2, F3)
where
    F1: FnOnce() -> T1 + Send,
    F2: FnOnce() -> T2 + Send,
    F3: FnOnce() -> T3 + Send,
    T1: Send,
    T2: Send,
    T3: Send,
{
    type Output = (T1, T2, T3);

    fn par_run(self) -> Self::Output {
        let (f1, f2, f3) = self;
        let (v1, (v2, v3)) = rayon::join(f1, || rayon::join(f2, f3));
        (v1, v2, v3)
    }
}

impl<F1, F2, F3, F4, T1, T2, T3, T4> ParRun for (F1, F2, F3, F4)
where
    F1: FnOnce() -> T1 + Send,
    F2: FnOnce() -> T2 + Send,
    F3: FnOnce() -> T3 + Send,
    F4: FnOnce() -> T4 + Send,
    T1: Send,
    T2: Send,
    T3: Send,
    T4: Send,
{
    type Output = (T1, T2, T3, T4);

    fn par_run(self) -> Self::Output {
        let (f1, f2, f3, f4) = self;
        let ((v1, v2), (v3, v4)) = rayon::join(|| rayon::join(f1, f2), || rayon::join(f3, f4));
        (v1, v2, v3, v4)
    }
}

impl<F1, F2, F3, F4, F5, T1, T2, T3, T4, T5> ParRun for (F1, F2, F3, F4, F5)
where
    F1: FnOnce() -> T1 + Send,
    F2: FnOnce() -> T2 + Send,
    F3: FnOnce() -> T3 + Send,
    F4: FnOnce() -> T4 + Send,
    F5: FnOnce() -> T5 + Send,
    T1: Send,
    T2: Send,
    T3: Send,
    T4: Send,
    T5: Send,
{
    type Output = (T1, T2, T3, T4, T5);

    fn par_run(self) -> Self::Output {
        let (f1, f2, f3, f4, f5) = self;
        let ((v1, v2), (v3, (v4, v5))) = rayon::join(
            || rayon::join(f1, f2),
            || rayon::join(f3, || rayon::join(f4, f5)),
        );
        (v1, v2, v3, v4, v5)
    }
}
