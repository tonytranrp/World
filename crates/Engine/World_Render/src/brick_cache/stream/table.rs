//! A map from node keys to values that stores the keys below a level as
//! paths in the tree instead of hashing them.
//!
//! Keys below `split` are kept per ancestor at `split` in a small tree of
//! sparse arrays, laid out like the 64-tree itself: a node holds its
//! children's values contiguously, found by the rank of their slot in a
//! 64-bit mask, and the nodes of its children that have values below them
//! the same way. No key is stored and no slot is left empty, so a value
//! costs its own size and a little array slack; siblings, which stage 1
//! visits together, share cache lines. Inserting or removing shifts at most
//! 32 siblings, so nothing ever grows or rehashes many entries at once (a
//! single hash table of 1.4 million entries once resized in one insert:
//! 282 ms on the floor Mac, decision 0012). A part that empties is dropped.
//!
//! Keys at or above `split` share an ordinary hash map, as do the parts.
//! The small hash maps this replaces cost 269 bytes per 112-byte residency
//! entry on the floor walks (a 16-byte key and a control byte per slot, and
//! slots about half full); this layout costs about 130 (decision 0014).

use std::collections::VecDeque;

use bevy::platform::collections::HashMap;

use super::key::NodeKey;

pub struct Table<V> {
    split: u32,
    top: HashMap<NodeKey, V>,
    parts: HashMap<NodeKey, Node<V>>,
    len: usize,
}

/// The values below one node, by slot path.
#[derive(Clone)]
struct Node<V> {
    /// Slots of the children that have values.
    has: u64,
    /// Their values, in slot order.
    values: VecDeque<V>,
    /// Slots of the children with values below them.
    below: u64,
    /// Their nodes, in slot order.
    subs: VecDeque<Node<V>>,
}

impl<V> Default for Node<V> {
    fn default() -> Self {
        Self {
            has: 0,
            values: VecDeque::new(),
            below: 0,
            subs: VecDeque::new(),
        }
    }
}

/// The index of `slot` among the set bits of `mask`.
fn rank(mask: u64, slot: u32) -> usize {
    (mask & ((1u64 << slot) - 1)).count_ones() as usize
}

/// Inserts at `i`, growing the capacity by about a quarter rather than
/// doubling it, so a full array of large values carries little slack. The
/// arrays are ring buffers, so inserting or removing shifts the shorter
/// side: children inserted or removed in slot order (an expansion, a
/// collapse, freeing a buried subtree) shift nothing.
fn insert_at<T>(v: &mut VecDeque<T>, i: usize, x: T) {
    if v.len() == v.capacity() {
        v.reserve_exact((v.len() / 4).max(2));
    }
    v.insert(i, x);
}

/// Removes at `i` (which must exist), giving back capacity once half of it
/// is unused.
fn remove_at<T>(v: &mut VecDeque<T>, i: usize) -> T {
    let x = v.remove(i).expect("the rank of a set bit is in range");
    if v.capacity() > 4 && v.len() * 2 < v.capacity() {
        v.shrink_to(v.len() + v.len() / 4);
    }
    x
}

impl<V> Node<V> {
    fn is_empty(&self) -> bool {
        self.has == 0 && self.below == 0
    }

    fn sub(&self, slot: u32) -> Option<&Node<V>> {
        let bit = 1u64 << slot;
        (self.below & bit != 0).then(|| &self.subs[rank(self.below, slot)])
    }

    fn sub_mut(&mut self, slot: u32) -> Option<&mut Node<V>> {
        let bit = 1u64 << slot;
        (self.below & bit != 0).then(|| &mut self.subs[rank(self.below, slot)])
    }

    fn sub_or_insert(&mut self, slot: u32) -> &mut Node<V> {
        let bit = 1u64 << slot;
        let i = rank(self.below, slot);
        if self.below & bit == 0 {
            self.below |= bit;
            insert_at(&mut self.subs, i, Node::default());
        }
        &mut self.subs[i]
    }

    fn value(&self, slot: u32) -> Option<&V> {
        let bit = 1u64 << slot;
        (self.has & bit != 0).then(|| &self.values[rank(self.has, slot)])
    }

    fn value_mut(&mut self, slot: u32) -> Option<&mut V> {
        let bit = 1u64 << slot;
        (self.has & bit != 0).then(|| &mut self.values[rank(self.has, slot)])
    }

    fn put(&mut self, slot: u32, value: V) -> Option<V> {
        let bit = 1u64 << slot;
        let i = rank(self.has, slot);
        if self.has & bit != 0 {
            return Some(std::mem::replace(&mut self.values[i], value));
        }
        self.has |= bit;
        insert_at(&mut self.values, i, value);
        None
    }

    /// The node `depth` slots below a part's top holding `entries`, whose
    /// `paths` all pass through it, grouped as [`Part::from_grouped`]
    /// requires.
    fn grouped(paths: &[Path], entries: &[(NodeKey, V)], depth: usize) -> Self
    where
        V: Clone,
    {
        assert!(
            paths.iter().all(|p| p.len > depth),
            "entries grouped depth first, each key once"
        );
        // The runs of entries under each child slot, with the run's own
        // value (a path ending at this depth) at either end.
        let runs = || {
            let mut start = 0;
            std::iter::from_fn(move || {
                let slot = paths.get(start)?.slots[depth];
                let len = paths[start..]
                    .iter()
                    .take_while(|p| p.slots[depth] == slot)
                    .count();
                let run = start..start + len;
                start += len;
                let own = |i: usize| paths[i].len == depth + 1;
                let (value, below) = if own(run.end - 1) {
                    (Some(run.end - 1), run.start..run.end - 1)
                } else if own(run.start) {
                    (Some(run.start), run.start + 1..run.end)
                } else {
                    (None, run)
                };
                Some((slot, value, below))
            })
        };
        let (mut values, mut subs) = (0, 0);
        for (_, value, below) in runs() {
            values += usize::from(value.is_some());
            subs += usize::from(!below.is_empty());
        }
        let mut node = Self {
            has: 0,
            values: VecDeque::with_capacity(values),
            below: 0,
            subs: VecDeque::with_capacity(subs),
        };
        for (slot, value, below) in runs() {
            let bit = 1u64 << slot;
            assert!(
                (node.has | node.below) < bit,
                "siblings in ascending slot order, each run once"
            );
            if let Some(i) = value {
                node.has |= bit;
                node.values.push_back(entries[i].1.clone());
            }
            if !below.is_empty() {
                node.below |= bit;
                let sub = Self::grouped(&paths[below.clone()], &entries[below], depth + 1);
                node.subs.push_back(sub);
            }
        }
        node
    }

    /// Removes the value at `path` (slots from this node down, the last one
    /// the key's slot in its parent), dropping nodes that empty on the way
    /// back up.
    fn take(&mut self, path: &[u32]) -> Option<V> {
        match path {
            [] => None,
            [slot] => {
                let bit = 1u64 << slot;
                if self.has & bit == 0 {
                    return None;
                }
                let i = rank(self.has, *slot);
                self.has &= !bit;
                Some(remove_at(&mut self.values, i))
            }
            [slot, rest @ ..] => {
                let sub = self.sub_mut(*slot)?;
                let old = sub.take(rest);
                if sub.is_empty() {
                    let i = rank(self.below, *slot);
                    self.below &= !(1u64 << slot);
                    remove_at(&mut self.subs, i);
                }
                old
            }
        }
    }

    /// Every value below this node, whose key is `key`, depth first in slot
    /// order.
    fn walk<'a>(&'a self, key: NodeKey, out: &mut dyn FnMut(NodeKey, &'a V)) {
        let mut has = self.has;
        let mut values = self.values.iter();
        while has != 0 {
            let slot = has.trailing_zeros();
            has &= has - 1;
            if let Some(v) = values.next() {
                out(key.child(slot), v);
            }
        }
        let mut below = self.below;
        let mut subs = self.subs.iter();
        while below != 0 {
            let slot = below.trailing_zeros();
            below &= below - 1;
            if let Some(n) = subs.next() {
                n.walk(key.child(slot), out);
            }
        }
    }
}

/// The slots from a part's top node down to `key`: at most `split - 1`.
#[derive(Clone, Copy)]
struct Path {
    slots: [u32; 8],
    len: usize,
}

impl Path {
    fn of(key: &NodeKey, split: u32) -> Self {
        let mut slots = [0; 8];
        let len = (split - key.level()) as usize;
        for (i, s) in slots[..len].iter_mut().enumerate() {
            // From the top: the ancestor just below the part's node first.
            *s = key.ancestor(split - 1 - i as u32).slot_in_parent();
        }
        Self { slots, len }
    }

    fn slots(&self) -> &[u32] {
        &self.slots[..self.len]
    }
}

/// The values below one node at a table's split level, built apart from
/// any table (on another thread, say) and handed over whole with
/// [`Table::insert_part`]: one hash map insertion, however many values.
/// Keys are given under any node at the split level; only their path below
/// it counts.
#[derive(Clone)]
pub struct Part<V> {
    split: u32,
    node: Node<V>,
    len: usize,
}

impl<V> Part<V> {
    /// An empty part for a table split at `split`.
    pub fn new(split: u32) -> Self {
        assert!((1..=8).contains(&split), "split level {split}");
        Self {
            split,
            node: Node::default(),
            len: 0,
        }
    }

    /// Inserts or replaces the value of `key`, a node below the split level;
    /// returns the value replaced. Keys inserted in slot order, children
    /// before their parent, only ever append. Parts are built with
    /// [`from_grouped`](Self::from_grouped); this is the tests' reference.
    #[cfg(test)]
    pub fn insert(&mut self, key: NodeKey, value: V) -> Option<V> {
        assert!(
            key.level() < self.split,
            "a part holds keys below its split"
        );
        let path = Path::of(&key, self.split);
        let (last, down) = path
            .slots()
            .split_last()
            .expect("a key below the split has a path");
        let mut n = &mut self.node;
        for &s in down {
            n = n.sub_or_insert(s);
        }
        let old = n.put(*last, value);
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    /// A part holding exactly `entries`, built with every array allocated
    /// once at its final length (inserting one by one grows each array
    /// fifteen times on the way to 64 values). The entries must be grouped
    /// depth first: everything below a node next to each other, siblings in
    /// ascending slot order, and a node's own value just before or just
    /// after its descendants (slot order with children before their
    /// parent is one such order). Panics otherwise, or for a key at or
    /// above the split.
    pub fn from_grouped(split: u32, entries: &[(NodeKey, V)]) -> Self
    where
        V: Clone,
    {
        let mut part = Self::new(split);
        let paths: Vec<Path> = entries
            .iter()
            .map(|(k, _)| {
                assert!(k.level() < split, "a part holds keys below its split");
                Path::of(k, split)
            })
            .collect();
        part.node = Node::grouped(&paths, entries, 0);
        part.len = entries.len();
        part
    }

    /// The value of `key`, by its path below the split level.
    pub fn get(&self, key: &NodeKey) -> Option<&V> {
        if key.level() >= self.split {
            return None;
        }
        let (n, slot) = Table::holder(&self.node, &Path::of(key, self.split))?;
        n.value(slot)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<V> Table<V> {
    /// An empty table whose keys below level `split` are kept by their
    /// ancestor at `split`.
    pub fn new(split: u32) -> Self {
        assert!((1..=8).contains(&split), "split level {split}");
        Self {
            split,
            top: HashMap::default(),
            parts: HashMap::default(),
            len: 0,
        }
    }

    fn part_of(&self, key: &NodeKey) -> Option<(NodeKey, Path)> {
        (key.level() < self.split).then(|| (key.ancestor(self.split), Path::of(key, self.split)))
    }

    /// The node holding `key`'s value (its parent), and `key`'s slot in it.
    fn holder<'a>(node: &'a Node<V>, path: &Path) -> Option<(&'a Node<V>, u32)> {
        let (last, down) = path.slots().split_last()?;
        let mut n = node;
        for &s in down {
            n = n.sub(s)?;
        }
        Some((n, *last))
    }

    pub fn get(&self, key: &NodeKey) -> Option<&V> {
        match self.part_of(key) {
            Some((part, path)) => {
                let (n, slot) = Self::holder(self.parts.get(&part)?, &path)?;
                n.value(slot)
            }
            None => self.top.get(key),
        }
    }

    pub fn get_mut(&mut self, key: &NodeKey) -> Option<&mut V> {
        match self.part_of(key) {
            Some((part, path)) => {
                let (last, down) = path.slots().split_last()?;
                let mut n = self.parts.get_mut(&part)?;
                for &s in down {
                    n = n.sub_mut(s)?;
                }
                n.value_mut(*last)
            }
            None => self.top.get_mut(key),
        }
    }

    pub fn contains_key(&self, key: &NodeKey) -> bool {
        self.get(key).is_some()
    }

    /// Inserts or replaces; returns the value replaced.
    pub fn insert(&mut self, key: NodeKey, value: V) -> Option<V> {
        let old = match self.part_of(&key) {
            Some((part, path)) => {
                let (last, down) = path
                    .slots()
                    .split_last()
                    .expect("a key below the split has a path");
                let mut n = self.parts.entry(part).or_default();
                for &s in down {
                    n = n.sub_or_insert(s);
                }
                n.put(*last, value)
            }
            None => self.top.insert(key, value),
        };
        if old.is_none() {
            self.len += 1;
        }
        old
    }

    pub fn remove(&mut self, key: &NodeKey) -> Option<V> {
        let old = match self.part_of(key) {
            Some((part, path)) => {
                let n = self.parts.get_mut(&part)?;
                let old = n.take(path.slots());
                if n.is_empty() {
                    self.parts.remove(&part);
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

    /// Removes every key below `part`, a node at the split level (not
    /// `part` itself); returns how many there were.
    pub fn remove_below(&mut self, part: &NodeKey) -> usize {
        if part.level() != self.split {
            return 0;
        }
        let Some(n) = self.parts.remove(part) else {
            return 0;
        };
        let mut count = 0;
        n.walk(*part, &mut |_, _| count += 1);
        self.len -= count;
        count
    }

    /// Replaces every key below `part`, a node at the split level, with the
    /// values of `values`: one hash map insertion. Returns how many keys
    /// were removed.
    pub fn insert_part(&mut self, part: NodeKey, values: Part<V>) -> usize {
        assert_eq!(part.level(), self.split, "a part goes at the split level");
        assert_eq!(values.split, self.split, "a part built for this split");
        let removed = self.remove_below(&part);
        if !values.node.is_empty() {
            self.len += values.len;
            self.parts.insert(part, values.node);
        }
        removed
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Entries in the largest hash map the table keeps (the top map or the
    /// map of parts): what one insertion may have to rehash at worst. The
    /// arrays inside a part never rehash.
    pub fn largest_part(&self) -> usize {
        self.top.len().max(self.parts.len())
    }

    pub fn clear(&mut self) {
        self.top.clear();
        self.parts.clear();
        self.len = 0;
    }

    /// Every entry, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeKey, &V)> {
        let mut below = Vec::new();
        for (part, n) in &self.parts {
            n.walk(*part, &mut |k, v| below.push((k, v)));
        }
        self.top.iter().map(|(k, v)| (*k, v)).chain(below)
    }

    pub fn keys(&self) -> impl Iterator<Item = NodeKey> {
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

    fn keys() -> Vec<NodeKey> {
        (0..4000u32)
            .map(|i| key((i % 6) as u8 + 1, i * 7 % 97, i * 13 % 89, i % 5))
            .collect()
    }

    #[test]
    fn behaves_like_one_map_and_drops_empty_parts() {
        for split in [1, 2, 3, 4] {
            let mut t = Table::new(split);
            let mut reference = std::collections::BTreeMap::new();
            let keys = keys();
            for (i, &k) in keys.iter().enumerate() {
                assert_eq!(t.insert(k, i), reference.insert(k, i));
            }
            for &k in keys.iter().step_by(3) {
                assert_eq!(t.remove(&k), reference.remove(&k));
                assert_eq!(t.remove(&k), None);
            }
            for &k in keys.iter().step_by(7) {
                if let Some(v) = t.get_mut(&k) {
                    *v += 1_000_000;
                }
                if let Some(v) = reference.get_mut(&k) {
                    *v += 1_000_000;
                }
            }
            assert_eq!(t.len(), reference.len());
            for &k in &keys {
                assert_eq!(t.get(&k), reference.get(&k));
                assert_eq!(t.contains_key(&k), reference.contains_key(&k));
            }
            let mut all: Vec<_> = t.iter().map(|(k, &v)| (k, v)).collect();
            all.sort_unstable();
            assert_eq!(all, reference.clone().into_iter().collect::<Vec<_>>());
            assert_eq!(t.values().count(), reference.len());
            for &k in &keys {
                t.remove(&k);
            }
            assert_eq!(t.len(), 0);
            assert!(
                t.parts.is_empty() && t.top.is_empty(),
                "no part left behind (split {split})"
            );
        }
    }

    #[test]
    fn removing_below_a_part_drops_exactly_its_keys() {
        let mut t = Table::new(3);
        let mut reference = std::collections::BTreeMap::new();
        for (i, k) in keys().into_iter().enumerate() {
            t.insert(k, i);
            reference.insert(k, i);
        }
        let part = key(3, 0, 0, 0);
        let below = reference
            .keys()
            .filter(|k| k.level() < 3 && k.ancestor(3) == part)
            .count();
        assert!(below > 0);
        assert_eq!(t.remove_below(&part), below);
        reference.retain(|k, _| !(k.level() < 3 && k.ancestor(3) == part));
        assert_eq!(t.len(), reference.len());
        let mut all: Vec<_> = t.iter().map(|(k, &v)| (k, v)).collect();
        all.sort_unstable();
        assert_eq!(all, reference.into_iter().collect::<Vec<_>>());
        assert_eq!(t.remove_below(&part), 0);
    }

    #[test]
    fn a_part_built_apart_reads_like_inserted_keys() {
        let split = 3;
        let mut direct = Table::new(split);
        let mut built = Table::new(split);
        let (here, elsewhere) = (key(3, 5, 6, 7), key(3, 0, 0, 0));
        let mut part = Part::new(split);
        let mut n = 0;
        for s in (0..64).step_by(3) {
            let child = here.child(s);
            for g in (0..64).step_by(5) {
                direct.insert(child.child(g), n);
                // The part is built under another node: only paths count.
                part.insert(elsewhere.child(s).child(g), n);
                n += 1;
            }
            direct.insert(child, n);
            part.insert(elsewhere.child(s), n);
            n += 1;
        }
        built.insert(here.child(1), 999);
        built.insert(key(4, 0, 0, 0), 1);
        direct.insert(key(4, 0, 0, 0), 1);
        assert_eq!(part.len(), n as usize);
        assert_eq!(part.get(&elsewhere.child(3)), part.get(&here.child(3)));
        assert_eq!(built.insert_part(here, part), 1, "the old key went");
        assert_eq!(built.len(), direct.len());
        let mut a: Vec<_> = built.iter().map(|(k, &v)| (k, v)).collect();
        let mut b: Vec<_> = direct.iter().map(|(k, &v)| (k, v)).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        assert_eq!(built.insert_part(here, Part::new(split)), n as usize);
        assert_eq!(built.len(), 1);
    }

    /// A part built from grouped entries, with parents after their
    /// children or before them, holds what inserting them one by one holds,
    /// with every array at its exact length; entries out of order are
    /// refused.
    #[test]
    fn a_part_from_grouped_entries_equals_one_built_by_inserting() {
        let split = 4;
        let top = key(4, 0, 0, 0);
        let (mut after, mut before) = (Vec::new(), Vec::new());
        let mut n = 0;
        for s in (0..64).step_by(7) {
            let child = top.child(s);
            let mut run = Vec::new();
            for g in (s % 3..64).step_by(9) {
                let grandchild = child.child(g);
                let mut leaves: Vec<_> = (g % 4..64)
                    .step_by(11)
                    .map(|l| (grandchild.child(l), n + l))
                    .collect();
                n += 100;
                if g % 2 == 0 {
                    leaves.push((grandchild, n));
                } else {
                    leaves.insert(0, (grandchild, n));
                }
                run.extend(leaves);
            }
            if s % 2 == 0 {
                run.push((child, n + 1));
            }
            after.extend(run.iter().copied());
            before.push((child, n + 1));
            before.extend(run.into_iter().filter(|&(k, _)| k != child));
            n += 2;
        }
        for entries in [&after, &before] {
            let built = Part::from_grouped(split, entries);
            let mut inserted = Part::new(split);
            for &(k, v) in entries.iter() {
                inserted.insert(k, v);
            }
            assert_eq!(built.len(), inserted.len());
            for &(k, v) in entries.iter() {
                assert_eq!(built.get(&k), Some(&v), "{k:?}");
            }
            let walk = |p: &Part<u32>| {
                let mut out = Vec::new();
                p.node.walk(top, &mut |k, &v| out.push((k, v)));
                out
            };
            assert_eq!(walk(&built), walk(&inserted));
            fn exact(n: &Node<u32>) -> bool {
                n.values.capacity() == n.values.len()
                    && n.subs.capacity() == n.subs.len()
                    && n.subs.iter().all(exact)
            }
            assert!(exact(&built.node));
        }
        let mut shuffled = after.clone();
        shuffled.swap(0, after.len() - 1);
        let refused = std::panic::catch_unwind(|| Part::from_grouped(split, &shuffled));
        assert!(refused.is_err(), "entries out of order");
    }

    #[test]
    fn arrays_give_back_what_removals_free() {
        let mut t = Table::new(3);
        let parent = key(2, 4, 4, 4);
        for s in 0..64 {
            t.insert(parent.child(s), [0u64; 8]);
        }
        for s in 0..60 {
            t.remove(&parent.child(s));
        }
        let part = &t.parts[&parent.parent()];
        let n = &part.subs[0];
        assert_eq!(n.values.len(), 4);
        assert!(n.values.capacity() <= 8, "{}", n.values.capacity());
    }
}
