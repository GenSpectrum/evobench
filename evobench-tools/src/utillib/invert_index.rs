use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

pub fn invert_index<T1: Ord + Clone, T2: Ord + Clone>(
    k_vs: &BTreeMap<T1, BTreeSet<T2>>,
) -> BTreeMap<T2, BTreeSet<T1>> {
    let mut v_ks: BTreeMap<T2, BTreeSet<T1>> = BTreeMap::new();
    for (k, vs) in k_vs {
        for v in vs {
            match v_ks.entry(v.clone()) {
                Entry::Vacant(vacant_entry) => {
                    let mut s = BTreeSet::new();
                    s.insert(k.clone());
                    vacant_entry.insert(s);
                }
                Entry::Occupied(mut occupied_entry) => {
                    occupied_entry.get_mut().insert(k.clone());
                }
            }
        }
    }
    v_ks
}

pub fn invert_index_by_ref<T1: Ord, T2: Ord>(
    k_vs: &BTreeMap<T1, BTreeSet<T2>>,
) -> BTreeMap<&T2, BTreeSet<&T1>> {
    let mut v_ks: BTreeMap<&T2, BTreeSet<&T1>> = BTreeMap::new();
    for (k, vs) in k_vs {
        for v in vs {
            match v_ks.entry(v) {
                Entry::Vacant(vacant_entry) => {
                    let mut s = BTreeSet::new();
                    s.insert(k);
                    vacant_entry.insert(s);
                }
                Entry::Occupied(mut occupied_entry) => {
                    occupied_entry.get_mut().insert(k);
                }
            }
        }
    }
    v_ks
}
