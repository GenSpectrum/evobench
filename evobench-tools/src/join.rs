use itertools::{EitherOrBoth, Itertools};

#[derive(Debug, PartialEq)]
pub struct KeyVal<K, V> {
    pub key: K,
    pub val: V,
}

pub fn keyval_inner_join_2<K: Ord, V1, V2>(
    a: impl IntoIterator<Item = KeyVal<K, V1>>,
    b: impl IntoIterator<Item = KeyVal<K, V2>>,
) -> impl Iterator<Item = KeyVal<K, (V1, V2)>> {
    a.into_iter()
        .merge_join_by(b.into_iter(), |a, b| a.key.cmp(&b.key))
        .filter_map(|eob| match eob {
            EitherOrBoth::Both(a, b) => Some(KeyVal {
                key: a.key,
                val: (a.val, b.val),
            }),
            EitherOrBoth::Left(_) => None,
            EitherOrBoth::Right(_) => None,
        })
}

/// Join any number of sequences of `KeyVal`, ordered by
/// `KeyVal.key`. Only keys present in all sequences are
/// preserved. I.e. the resulting sequence has `Vec`s of the same
/// number of elements as `sequences`. Returns None if `sequences` is
/// empty.
pub fn keyval_inner_join<'s, 'i, 'k, 'v, K: Ord + 'k, V: 'v>(
    sequences: &'s mut [Option<impl IntoIterator<Item = KeyVal<K, V>> + 'i>],
) -> Option<Box<dyn Iterator<Item = KeyVal<K, Vec<V>>> + 'i>>
where
    's: 'i,
    'k: 'i,
    'v: 'i,
{
    match sequences.len() {
        0 => None,
        1 => Some(Box::new(
            sequences[0]
                .take()
                .expect("checked")
                .into_iter()
                .map(|KeyVal { key, val }| KeyVal {
                    key,
                    val: vec![val],
                }),
        )),
        2 => Some(Box::new(
            keyval_inner_join_2(
                sequences[0].take().expect("checked"),
                sequences[1].take().expect("checked"),
            )
            .map(|KeyVal { key, val: (v1, v2) }| KeyVal {
                key,
                val: vec![v1, v2],
            }),
        )),
        n => {
            let (a, b) = sequences.split_at_mut(n / 2);
            let ar = keyval_inner_join(a).expect("at least 1 out of 3+");
            let br = keyval_inner_join(b).expect("at least 1 out of 3+");
            Some(Box::new(keyval_inner_join_2(ar, br).map(
                |KeyVal {
                     key,
                     val: (mut val1, mut val2),
                 }| {
                    val1.append(&mut val2);
                    KeyVal { key, val: val1 }
                },
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k<K, V>(k: K, v: V) -> KeyVal<K, V> {
        KeyVal { key: k, val: v }
    }

    fn seqs() -> (
        Vec<KeyVal<&'static str, i32>>,
        Vec<KeyVal<&'static str, i32>>,
        Vec<KeyVal<&'static str, i32>>,
    ) {
        (
            vec![k("a", 1), k("a2", 2), k("b", 3), k("t", 4), k("u", 5)],
            vec![
                k("a03", 10),
                k("a2", 20),
                k("b2", 30),
                k("t", 40),
                k("u", 50),
            ],
            vec![
                k("a01", 100),
                k("a2", 200),
                k("b3", 300),
                k("t", 400),
                k("u", 500),
                k("v", 600),
            ],
        )
    }

    #[test]
    fn t_2() {
        let (a, b, _c) = seqs();

        // Oh my, sharing is involved:
        let res = keyval_inner_join_2(
            a.iter().map(|KeyVal { key, val }| KeyVal { key, val }),
            b.iter().map(|KeyVal { key, val }| KeyVal { key, val }),
        )
        .collect::<Vec<_>>();
        assert_eq!(
            res,
            vec![k("a2", (2, 20)), k("t", (4, 40)), k("u", (5, 50))]
                .iter()
                .map(
                    |KeyVal {
                         key,
                         val: (val1, val2),
                     }| KeyVal {
                        key,
                        val: (val1, val2)
                    }
                )
                .collect::<Vec<_>>()
        );

        // Owned is easy:
        assert_eq!(
            keyval_inner_join_2(a, b).collect::<Vec<_>>(),
            vec![k("a2", (2, 20)), k("t", (4, 40)), k("u", (5, 50))]
        );
    }

    #[test]
    fn t_3() {
        let (a, b, c) = seqs();
        let r = keyval_inner_join(&mut [Some(a), Some(b), Some(c)])
            .expect("given inputs")
            .collect::<Vec<_>>();
        assert_eq!(
            r,
            vec![
                k("a2", vec![2, 20, 200]),
                k("t", vec![4, 40, 400]),
                k("u", vec![5, 50, 500])
            ]
        );
        let (a, b, c) = seqs();
        let d = vec![k("a", 1), k("a2", 2), k("b", 3), k("u", 5)];
        let mut v = [a, b, c, d].map(Some);
        let r = keyval_inner_join(&mut v)
            .expect("given inputs")
            .collect::<Vec<_>>();
        assert_eq!(
            r,
            vec![k("a2", vec![2, 20, 200, 2]), k("u", vec![5, 50, 500, 5])]
        );
    }
}
