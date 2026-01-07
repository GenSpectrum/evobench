//! A simple, heap allocation based hence slow, string key based tree
//! representation.

use std::collections::{btree_map::Entry, BTreeMap};

#[derive(Debug)]
pub struct Tree<'t, T> {
    pub value: Option<T>,
    pub children: BTreeMap<&'t str, Tree<'t, T>>,
}

impl<'key, T> Tree<'key, T> {
    pub fn new() -> Self {
        Self {
            value: None,
            children: Default::default(),
        }
    }

    pub fn get(&self, mut path: impl Iterator<Item = &'key str>) -> Option<&T> {
        if let Some(key) = path.next() {
            self.children.get(key)?.get(path)
        } else {
            self.value.as_ref()
        }
    }

    pub fn insert(&mut self, mut path: impl Iterator<Item = &'key str>, value: T) {
        if let Some(key) = path.next() {
            match self.children.entry(key) {
                Entry::Vacant(vacant_entry) => {
                    let mut node = Self {
                        value: None,
                        children: Default::default(),
                    };
                    node.insert(path, value);
                    vacant_entry.insert(node);
                }
                Entry::Occupied(mut occupied_entry) => {
                    occupied_entry.get_mut().insert(path, value);
                }
            }
        } else {
            self.value = Some(value);
        }
    }

    pub fn from_key_val(
        data: impl IntoIterator<Item = (impl IntoIterator<Item = &'key str>, T)>,
    ) -> Self {
        let mut top = Self::new();
        for (key, val) in data {
            top.insert(key.into_iter(), val);
        }
        top
    }

    pub fn into_map_values<U>(self, f: &impl Fn(T) -> U) -> Tree<'key, U> {
        let Self { value, children } = self;
        Tree {
            value: value.map(f),
            children: children
                .into_iter()
                .map(|(key, child)| (key, child.into_map_values(f)))
                .collect(),
        }
    }

    pub fn map_values<U>(&self, f: &impl Fn(&T) -> U) -> Tree<'key, U> {
        let Self { value, children } = self;
        Tree {
            value: value.as_ref().map(f),
            children: children
                .iter()
                .map(|(key, child)| (*key, child.map_values(f)))
                .collect(),
        }
    }

    /// Return a sequence of path, value pairs, sorted by path, the
    /// path built by joining the keys with `separator`.
    pub fn into_joined_key_val(self, separator: &str) -> Vec<(String, T)> {
        let mut path: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        into_joined_key_val(self, separator, &mut path, &mut out);
        out
    }
}

fn into_joined_key_val<'key, T>(
    tree: Tree<'key, T>,
    separator: &'key str,
    path: &mut Vec<&'key str>,
    out: &mut Vec<(String, T)>,
) {
    let Tree { value, children } = tree;
    if let Some(value) = value {
        out.push((path.join(separator), value));
    }
    for (key, child) in children.into_iter() {
        path.push(key);
        into_joined_key_val(child, separator, path, out);
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_forth_and_back() {
        let vals = &[("a", 2), ("a:b", 1), ("c:d", 3), ("d:e:f", 4), ("d", 5)];
        let mut tree = Tree::new();
        for (key, val) in vals {
            tree.insert(key.split(':'), *val);
        }
        let vals2 = tree.into_joined_key_val("--");

        let s = |s: &str| s.to_string();
        assert_eq!(
            vals2,
            [
                (s("a"), 2),
                (s("a--b"), 1),
                (s("c--d"), 3),
                (s("d"), 5),
                (s("d--e--f"), 4),
            ]
        );
    }
}
