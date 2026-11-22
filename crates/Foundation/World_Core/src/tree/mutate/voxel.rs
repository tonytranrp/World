//! Voxel writes, applied as one batch in tree order.

use super::brick::{self, Records};
use super::{Writer, branch::Children};
use crate::consts::TREE_FANOUT;
use crate::coords::path;
use crate::edit::{Edit, EditError};
use crate::material::MaterialId;
use crate::tree::node::leaf::{self, PackedLeaf, Record};
use crate::tree::node::{Child, Node, bit, slot_at, world_key, world_key_local};

/// One voxel write, in world-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalEdit {
    pub local: [u32; 3],
    pub material: MaterialId,
}

/// Leaves of one level-1 node that a splice rewrites at most; a node with
/// more changed leaves that cannot be patched in place is repacked from its
/// records.
const SPLICE_MAX: usize = 8;

/// Voxels of one brick (a level-1 node).
const BRICK_VOXELS: usize = 1 << 12;

/// The child a run of edits of a child of a node at `level` (2 or more)
/// makes when it names every voxel of that child (one edit per voxel, so
/// its length is the child's volume, `64^level`) with one material.
fn covered_fill(run: &[LocalEdit], level: u32) -> Option<Child> {
    if run.len() as u64 != 1 << (6 * level) {
        return None;
    }
    let material = run[0].material;
    run.iter()
        .all(|e| e.material == material)
        .then(|| Child::filled(material))
}

/// Batches at least this long are sorted by radix; shorter ones by
/// comparison, which needs no counting pass.
const RADIX_MIN: usize = 256;

/// Bits of one radix digit: 2,048 counters, which stay in the L1 cache.
const RADIX_DIGIT: u32 = 8;

/// `edits` in world-local form and tree order (by `node::world_key`), one
/// per voxel: the last edit of the batch for each voxel. Fails on the first
/// edit outside the world.
///
/// A batch's keys share every bit above the highest bit in which any two
/// differ, so a radix sort needs to pass only over the bits that differ:
/// three passes for a crater two metres across. The key, shorn of the
/// shared high bits, and the material pack into one `u64`; the sort is
/// stable, so among edits of one voxel the batch order stays.
pub(crate) fn tree_order(edits: &[Edit]) -> Result<Vec<LocalEdit>, EditError> {
    let mut items = Vec::with_capacity(edits.len());
    let (mut any, mut all) = (0u64, u64::MAX);
    for edit in edits {
        let pos = edit.pos;
        let local = path::world_local(pos).ok_or(EditError::OutOfWorld { pos })?;
        let key = world_key(local);
        any |= key;
        all &= key;
        items.push(key);
    }
    let varying = any ^ all;
    let high = u64::BITS - varying.leading_zeros();
    let mut out = Vec::with_capacity(items.len());
    if items.len() < RADIX_MIN || high > u64::BITS - 16 {
        let mut keyed: Vec<(u64, MaterialId)> = items
            .iter()
            .zip(edits)
            .map(|(&key, edit)| (key, edit.material))
            .collect();
        keyed.sort_by_key(|&(key, _)| key);
        for (i, &(key, material)) in keyed.iter().enumerate() {
            if keyed.get(i + 1).is_none_or(|next| next.0 != key) {
                out.push(LocalEdit {
                    local: world_key_local(key),
                    material,
                });
            }
        }
        return Ok(out);
    }
    let low = (1u64 << high) - 1;
    for (item, edit) in items.iter_mut().zip(edits) {
        *item = (*item & low) << 16 | u64::from(edit.material.0);
    }
    radix_sort(&mut items, 16 + varying.trailing_zeros(), 16 + high);
    let shared = all & !low;
    for (i, &item) in items.iter().enumerate() {
        if items.get(i + 1).is_none_or(|next| next >> 16 != item >> 16) {
            out.push(LocalEdit {
                local: world_key_local(item >> 16 | shared),
                material: MaterialId(item as u16),
            });
        }
    }
    Ok(out)
}

/// Sorts `items` stably by their bits `from..to` (least significant digit
/// first); the bits at and above `to` are zero.
fn radix_sort(items: &mut Vec<u64>, from: u32, to: u32) {
    let width = to.saturating_sub(from);
    if width == 0 {
        return;
    }
    let passes = width.div_ceil(RADIX_DIGIT) as usize;
    let digit = width.div_ceil(passes as u32);
    let radix = 1usize << digit;
    let mask = radix as u64 - 1;
    let mut counts = vec![0usize; passes * radix];
    for &item in items.iter() {
        for pass in 0..passes {
            let d = (item >> (from + pass as u32 * digit) & mask) as usize;
            counts[pass * radix + d] += 1;
        }
    }
    let mut scratch = vec![0u64; items.len()];
    for pass in 0..passes {
        let shift = from + pass as u32 * digit;
        let starts = &mut counts[pass * radix..(pass + 1) * radix];
        let mut sum = 0;
        for c in starts.iter_mut() {
            let n = *c;
            *c = sum;
            sum += n;
        }
        for &item in items.iter() {
            let d = (item >> shift & mask) as usize;
            scratch[starts[d]] = item;
            starts[d] += 1;
        }
        std::mem::swap(items, &mut scratch);
    }
}

impl Writer<'_> {
    /// Applies `edits` (in tree order, one per voxel, all inside the child's
    /// region) to `child`, a child at `level`. Returns the child that
    /// replaces it, or `None` if no voxel changed.
    pub(super) fn set_voxels(
        &mut self,
        child: Child,
        level: u32,
        edits: &[LocalEdit],
        changed: &mut impl FnMut(LocalEdit),
    ) -> Option<Child> {
        if child == Child::NotResident {
            self.refused = true;
            return None;
        }
        if let Child::Air | Child::Uniform(_) = child
            && edits.iter().all(|e| Child::filled(e.material) == child)
        {
            return None;
        }
        if self.at_chunk_boundary(level) {
            return self.in_chunk(child, edits[0].local, |w, local| {
                w.set_voxels(local, level, edits, changed)
            });
        }
        if level == 1 {
            return self.set_leaves(child, edits, changed);
        }
        let existing = self.existing(child);
        // Decoded only once some child is replaced by a different one.
        let mut children: Option<Children> = None;
        let mut any = false;
        for run in edits.chunk_by(|a, b| slot_at(a.local, level) == slot_at(b.local, level)) {
            let slot = slot_at(run[0].local, level);
            let old = match (&children, existing) {
                (Some(children), _) => children[slot as usize],
                (None, Some((_, node))) => self.child(node, slot),
                // Air or uniform: every child is the same fill.
                (None, None) => child,
            };
            // A run that names every voxel of the child with one material
            // (a crater's inside) replaces it whole, without a write per
            // voxel. Chunks are entered as usual.
            let new = match covered_fill(run, level) {
                Some(fill) if old != Child::NotResident && !self.at_chunk_boundary(level - 1) => {
                    if old == fill {
                        continue;
                    }
                    self.report_filled(old, level - 1, run, fill, changed);
                    self.free_subtree(old, level - 1);
                    fill
                }
                _ => match self.set_voxels(old, level - 1, run, changed) {
                    Some(new) => new,
                    None => continue,
                },
            };
            any = true;
            if new == old {
                continue;
            }
            if children.is_none()
                && let Some((_, node)) = existing
                && self.patch_child(node, slot, new)
            {
                continue;
            }
            children.get_or_insert_with(|| self.children(child))[slot as usize] = new;
        }
        if !any {
            return None;
        }
        Some(match children {
            // Every changed child was rebuilt in place: this node's words stand.
            None => child,
            Some(children) => self.write(existing, level, &super::branch::pack(&children)),
        })
    }

    /// Reports to `changed` an edit of every brick under `old`, a child at
    /// `level` (1 or more), that does not already hold `fill`: `run` names
    /// every voxel of `old` in tree order, so the voxels of each of its
    /// children are one equal share of it, in slot order.
    fn report_filled(
        &self,
        old: Child,
        level: u32,
        run: &[LocalEdit],
        fill: Child,
        changed: &mut impl FnMut(LocalEdit),
    ) {
        if old == fill {
            return;
        }
        match old {
            Child::Node(_) if level >= 2 => {
                let share = run.len() / TREE_FANOUT as usize;
                for (slot, child) in self.children(old).into_iter().enumerate() {
                    let part = &run[slot * share..(slot + 1) * share];
                    self.report_filled(child, level - 1, part, fill, changed);
                }
            }
            // A brick node is never one material.
            Child::Node(_) => changed(run[0]),
            // Every brick of a different fill changes.
            _ => {
                for brick in run.chunks(BRICK_VOXELS) {
                    changed(brick[0]);
                }
            }
        }
    }

    /// [`set_voxels`](Self::set_voxels) for a level-1 child, whose children
    /// are leaf records.
    fn set_leaves(
        &mut self,
        child: Child,
        edits: &[LocalEdit],
        changed: &mut impl FnMut(LocalEdit),
    ) -> Option<Child> {
        let existing = self.existing(child);
        let node = existing.map(|(_, node)| node);
        // A changed leaf is rewritten in place where it keeps its stored
        // length; the others wait, in slot order, for one splice (or one
        // repack) of the whole node once every leaf is known. Patches only
        // write their own leaf's words, so leaves read later, the splice and
        // the repack all see them.
        let mut pending = [(0u32, Record::AIR); SPLICE_MAX];
        let mut count = 0;
        // Once more leaves wait than the splice takes, or at once for an air
        // or uniform child (split, so packed anyway): every record.
        let mut records: Option<Records> = match node {
            None => Some(self.records(child)),
            Some(_) => None,
        };
        let mut any = false;
        for run in edits.chunk_by(|a, b| slot_at(a.local, 1) == slot_at(b.local, 1)) {
            let slot = slot_at(run[0].local, 1);
            let old = match &records {
                Some(records) => records[slot as usize],
                None => self.record(child, node, slot),
            };
            let Some(new) = self.edit_leaf(old, run, changed) else {
                continue;
            };
            any = true;
            if new == old {
                continue;
            }
            if let Some(records) = &mut records {
                records[slot as usize] = new;
                continue;
            }
            if let Some(node) = node
                && self.patch_record(node, slot, new)
            {
                continue;
            }
            if count < SPLICE_MAX {
                pending[count] = (slot, new);
                count += 1;
                continue;
            }
            let all = records.insert(self.records(child));
            for &(slot, new) in &pending[..count] {
                all[slot as usize] = new;
            }
            all[slot as usize] = new;
        }
        if !any {
            return None;
        }
        if let Some(records) = records {
            return Some(self.write(existing, 1, &brick::pack(&records)));
        }
        if count == 0 {
            // Every changed leaf was rewritten in place: the mask stands.
            return Some(child);
        }
        Some(self.write_leaves(child, existing, &pending[..count]))
    }

    /// Writes level-1 `child` (whose node, if it is one, is `existing`) with
    /// the leaves of `changes` replaced: one splice of the stored block, or a
    /// repack from every record where a splice cannot keep the node
    /// canonical. Out of line: the edit loop stays small.
    #[inline(never)]
    fn write_leaves(
        &mut self,
        child: Child,
        existing: Option<(u32, Node)>,
        changes: &[(u32, Record)],
    ) -> Child {
        if let Some((_, node)) = existing
            && let Some(packed) = self.splice(node, changes)
        {
            #[cfg(debug_assertions)]
            {
                let mut records: Records = self.records(child);
                for &(slot, new) in changes {
                    records[slot as usize] = new;
                }
                debug_assert_eq!(packed, brick::pack(&records), "a splice packs as a repack");
            }
            return self.write(existing, 1, &packed);
        }
        let mut records: Records = self.records(child);
        for &(slot, new) in changes {
            records[slot as usize] = new;
        }
        self.write(existing, 1, &brick::pack(&records))
    }

    /// Applies one leaf's edits. Returns its new record, or `None` if no
    /// voxel changed.
    fn edit_leaf(
        &mut self,
        old: Record,
        edits: &[LocalEdit],
        changed: &mut impl FnMut(LocalEdit),
    ) -> Option<Record> {
        // Most edits add or remove voxels of a leaf's one material: then the
        // new record is the old one with mask bits flipped.
        let single = match old.material() {
            Some(m) => Some(m),
            None if old.mask == 0 => edits.iter().map(|e| e.material).find(|m| !m.is_air()),
            None => None,
        };
        if let Some(m) = single
            && edits.iter().all(|e| e.material.is_air() || e.material == m)
        {
            let mut mask = old.mask;
            let mut first = None;
            for &edit in edits {
                let bit = bit(slot_at(edit.local, 0));
                let was = if mask & bit != 0 { m } else { MaterialId::AIR };
                if was != edit.material {
                    first.get_or_insert(edit);
                    mask ^= bit;
                }
            }
            changed(first?);
            let new = if mask == 0 {
                Record::AIR
            } else {
                Record::single(mask, m)
            };
            return Some(self.store_leaf(old, PackedLeaf::Inline(new)));
        }

        let mut voxels = self.leaf_voxels(old);
        let mut any = false;
        for &edit in edits {
            let voxel = &mut voxels[slot_at(edit.local, 0) as usize];
            if *voxel != edit.material {
                if !any {
                    changed(edit);
                }
                *voxel = edit.material;
                any = true;
            }
        }
        any.then(|| self.store_leaf(old, leaf::encode(&voxels)))
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, tree_order};
    use crate::coords::path;
    use crate::tree::node::path_key;
    use crate::tree::{NodeRef, VoxelTree};
    use crate::{MaterialId, VoxelPos};

    /// Tree order the plain way: a stable sort by path key, the last edit of
    /// each voxel kept.
    fn reference(edits: &[Edit]) -> Vec<([u32; 3], MaterialId)> {
        let mut keyed: Vec<(u64, [u32; 3], MaterialId)> = edits
            .iter()
            .map(|e| {
                let local = path::world_local(e.pos).unwrap();
                (
                    path_key(local, crate::consts::TREE_DEPTH),
                    local,
                    e.material,
                )
            })
            .collect();
        keyed.sort_by_key(|k| k.0);
        let mut out: Vec<(u64, [u32; 3], MaterialId)> = Vec::new();
        for k in keyed {
            match out.last_mut() {
                Some(last) if last.0 == k.0 => *last = k,
                _ => out.push(k),
            }
        }
        out.into_iter().map(|(_, l, m)| (l, m)).collect()
    }

    #[test]
    fn tree_order_sorts_and_keeps_the_last_edit_of_each_voxel() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |n: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u32 % n
        };
        // Small and large batches (comparison and radix), close together
        // and spread over the world (the radix sort's fallback), with
        // repeated voxels.
        for (count, spread, origin) in [
            (10, 8, 0),
            (300, 8, 1_000),
            (5_000, 40, -77_777),
            (5_000, 1 << 18, 0),
            (40_000, 300, 123_456),
        ] {
            let edits: Vec<Edit> = (0..count)
                .map(|_| {
                    let mut c = |o: i32| o + next(spread) as i32 - (spread / 2) as i32;
                    Edit::new(
                        VoxelPos::new(c(origin), c(-origin), c(origin / 3)),
                        MaterialId(next(5) as u16),
                    )
                })
                .collect();
            let got: Vec<_> = tree_order(&edits)
                .unwrap()
                .into_iter()
                .map(|e| (e.local, e.material))
                .collect();
            assert_eq!(got, reference(&edits), "{count} edits over {spread}");
        }
        assert!(tree_order(&[]).unwrap().is_empty());
        let one = VoxelPos::new(3, 4, 5);
        let same: Vec<Edit> = (0..1_000)
            .map(|i| Edit::new(one, MaterialId(i % 7)))
            .collect();
        let got = tree_order(&same).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].material, MaterialId(999 % 7));
    }

    #[test]
    fn a_write_inside_a_uniform_region_splits_down_to_a_leaf() {
        let mut tree = VoxelTree::new();
        tree.fill_box(crate::VoxelBox::WORLD, MaterialId(1))
            .unwrap();
        assert!(matches!(tree.root(), NodeRef::Uniform(MaterialId(1))));

        let p = VoxelPos::new(5, -6, 7);
        assert_eq!(tree.set(p, MaterialId(2)), Ok(true));
        let s = tree.stats();
        assert_eq!((s.branches, s.leaves), (9, 1), "one node per level");
        assert_eq!(tree.get(p), MaterialId(2));
        assert_eq!(tree.get(VoxelPos::new(5, -6, 8)), MaterialId(1));

        assert_eq!(tree.set(p, MaterialId(1)), Ok(true));
        assert!(matches!(tree.root(), NodeRef::Uniform(MaterialId(1))));
        let s = tree.stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
    }

    #[test]
    fn filling_a_leaf_voxel_by_voxel_collapses_it_into_a_uniform_child() {
        let mut tree = VoxelTree::new();
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    tree.set(VoxelPos::new(x, y, z), MaterialId(3)).unwrap();
                }
            }
        }
        let s = tree.stats();
        assert_eq!(s.leaves, 0, "a full leaf of one material is not a leaf");
        assert_eq!(s.branches, 9);
        // Eight child words, then one three-word record for the uniform leaf.
        assert_eq!(s.words, 8 + 3);
    }
}
