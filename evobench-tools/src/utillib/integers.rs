// XX again, had something like this somewhere (did I do it wrongly there?) (ah stats? where?)
pub fn rounding_integer_division(a: usize, b: usize) -> usize {
    let a = u128::try_from(a).expect("always works");
    let b = u128::try_from(b).expect("always works");
    let usize_max = u128::try_from(usize::MAX).expect("always works");

    let r = (a * usize_max + (b >> 1) * usize_max) / (b * usize_max);
    usize::try_from(r).expect("always fits")
}

#[test]
fn t_rounding_integer_division() {
    let t = rounding_integer_division;
    //                                 multiplying back:
    assert_eq!(t(33126, 1), 33126); // 33126
    assert_eq!(t(33126, 2), 16563); // 33126
    assert_eq!(t(33126, 3), 11042); // 33126
    assert_eq!(t(33126, 4), 8282); //  33128
    assert_eq!(t(33126, 9464), 4); //  37856
    assert_eq!(t(33126, 9465), 3); //  28395
    assert_eq!(t(33126, 12000), 3); // 36000
    assert_eq!(t(33126, 13563), 2); // 27126
    assert_eq!(t(33126, 16563), 2); // 33126
    assert_eq!(t(33126, 22083), 2); // 44166
    assert_eq!(t(33126, 22084), 2); // 44168
    assert_eq!(t(33126, 22085), 1); // 22085 -- the lowest possible
    assert_eq!(t(33126, 33126), 1); // 33126
}
