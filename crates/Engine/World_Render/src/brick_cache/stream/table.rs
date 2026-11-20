//! A map from node keys to values, split into many small maps so that no
//! single hash table ever grows or rehashes a million entries at once.
//!
//! Keys below `split` live in a small map per ancestor at `split` (at most
//! `64 + 64^2` keys each for a split of 3); the others share a top map. A
//! small map that empties is dropped, so deletions leave no tombstones
//! behind, and growing or rehashing one costs at most its own few thousand
//! entries. A single table of 1.4 million entries, with the tombstones a
//! node pool pressure collapse leaves, once resized in one insert: 282 ms
//! on the floor Mac (decision 0012).

use bevy::platform::collections::HashMap;

use super::key::NodeKey;

pub struct Table<V> {
    split: u32,
    top: HashMap<NodeKey, V>,
    groups: HashMap<NodeKey, HashMap<NodeKey, V>>,
    len: usize,
}

impl<V> Table<V> {
    /// An empty table whose keys below level `split` are grouped by their
    /// ancestor at `split`.
    pub fn new(split: u32) -> Self {
        Self {
            split,
            top: HashMap::default(),
            groups: HashMap::default(),
            len: 0,
        }
    }

    fn group_of(&self, key: &NodeKey) -> Option<NodeKey> {
        (key.level() < self.split).then(|| key.ancestor(self.split))
    }

    pub fn get(&self, key: &NodeKey) -> Option<&V> {
        match self.group_of(key) {
            Some(g) => self.groups.get(&g)?.get(key),
            None => self.top.get(key),
        }
    }

    pub fn get_mut(&mut self, key: &NodeKey) -> Option<&mut V> {
        match self.group_of(key) {
            Some(g) => self.groups.get_mut(&g)?.get_mut(key),
            None => self.top.get_mut(key),
        }
    }

    pub fn contains_key(&self, key: &NodeKey) -> bool {
        self.get(key).is_some()
    }

    /// Inserts or replaces; returns the value replaced.
    pub fn insert(&mut self, key: NodeKey, value: V) -> Option<V> {
        let old = match self.group_of(&key) {
            Some(g) => self.groups.entry(g).or_default().insert(key, value),
            None => self.top.insert(key, value),
        };
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    pub fn remove(&mut self, key: &NodeKey) -> Option<V> {
        let old = match self.group_of(key) {
            Some(g) => {
                let map = self.groups.get_mut(&g)?;
                let old = map.remove(key);
                if map.is_empty() {
                    self.groups.remove(&g);
                }
                old
            }
            None => self.top.remove(key),
        };
        if old.is_some() {
            self.len -= 1;
        }
        old
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Entries in the largest of the maps the table is split into.
    pub fn largest_part(&self) -> usize {
        self.groups
            .values()
            .map(HashMap::len)
            .chain([self.top.len()])
            .max()
            .unwrap_or(0)
    }

    pub fn clear(&mut self) {
        self.top.clear();
        self.groups.clear();
        self.len = 0;
    }

    /// Every entry, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&NodeKey, &V)> {
        self.top.iter().chain(self.groups.values().flatten())
    }

    pub fn keys(&self) -> impl Iterator<Item = &NodeKey> {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
}

impl<V> std::ops::Index<&NodeKey> for Table<V> {
    type Output = V;

    fn index(&self, key: &NodeKey) -> &V {
        self.get(key).expect("no entry for the key")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(level: u8, x: u32, y: u32, z: u32) -> NodeKey {
        NodeKey { level, x, y, z }
    }

    #[test]
    fn behaves_like_one_map_and_drops_empty_groups() {
        let mut t = Table::new(3);
        let mut reference = std::collections::BTreeMap::new();
        let keys: Vec<NodeKey> = (0..2000u32)
            .map(|i| key((i % 6) as u8 + 1, i * 7 % 97, i * 13 % 89, i % 5))
            .collect();
        for (i, &k) in keys.iter().enumerate() {
            assert_eq!(t.insert(k, i), reference.insert(k, i));
        }
        for &k in keys.iter().step_by(3) {
            assert_eq!(t.remove(&k), reference.remove(&k));
        }
        assert_eq!(t.len(), reference.len());
        for &k in &keys {
            assert_eq!(t.get(&k), reference.get(&k));
        }
        let mut all: Vec<_> = t.iter().map(|(&k, &v)| (k, v)).collect();
        all.sort_unstable();
        assert_eq!(all, reference.into_iter().collect::<Vec<_>>());
        for &k in &keys {
            t.remove(&k);
        }
        assert_eq!(t.len(), 0);
        assert!(
            t.groups.is_empty() && t.top.is_empty(),
            "no group left behind"
        );
    }
}
