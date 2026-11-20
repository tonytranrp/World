//! Voxel writes, applied as one batch in tree order.

use super::brick::{self, Records};
use super::{Writer, branch::Children};
use crate::material::MaterialId;
use crate::tree::node::leaf::{self, PackedLeaf, Record};
use crate::tree::node::{Child, bit, slot_at};

/// One voxel write, in world-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalEdit {
    pub local: [u32; 3],
    pub material: MaterialId,
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
        changed: &mut impl FnMut(LocalEdit, MaterialId),
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
            return self.in_chunk(child, |w, local| w.set_voxels(local, level, edits, changed));
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
            let Some(new) = self.set_voxels(old, level - 1, run, changed) else {
                continue;
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
            Some(children) => self.write(existing, level, super::branch::pack(&children)),
        })
    }

    /// [`set_voxels`](Self::set_voxels) for a level-1 child, whose children
    /// are leaf records.
    fn set_leaves(
        &mut self,
        child: Child,
        edits: &[LocalEdit],
        changed: &mut impl FnMut(LocalEdit, MaterialId),
    ) -> Option<Child> {
        let existing = self.existing(child);
        let node = existing.map(|(_, node)| node);
        // Decoded only once some leaf's record changes.
        let mut records: Option<Records> = None;
        let mut any = false;
        for run in edits.chunk_by(|a, b| slot_at(a.local, 1) == slot_at(b.local, 1)) {
            let slot = slot_at(run[0].local, 1) as usize;
            let old = match &records {
                Some(records) => records[slot],
                None => self.record(child, node, slot as u32),
            };
            let Some(new) = self.edit_leaf(old, run, changed) else {
                continue;
            };
            any = true;
            if new == old {
                continue;
            }
            if records.is_none()
                && let Some(node) = node
                && self.patch_record(node, slot as u32, new)
            {
                continue;
            }
            records.get_or_insert_with(|| self.records(child))[slot] = new;
        }
        if !any {
            return None;
        }
        Some(match records {
            // Every changed leaf was rewritten in place: the mask stands.
            None => child,
            Some(records) => self.write(existing, 1, brick::pack(&records)),
        })
    }

    /// Applies one leaf's edits. Returns its new record, or `None` if no
    /// voxel changed.
    fn edit_leaf(
        &mut self,
        old: Record,
        edits: &[LocalEdit],
        changed: &mut impl FnMut(LocalEdit, MaterialId),
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
            for &edit in edits {
                let bit = bit(slot_at(edit.local, 0));
                let was = if mask & bit != 0 { m } else { MaterialId::AIR };
                if was != edit.material {
                    changed(edit, was);
                    mask ^= bit;
                }
            }
            if mask == old.mask {
                return None;
            }
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
                changed(edit, *voxel);
                *voxel = edit.material;
                any = true;
            }
        }
        any.then(|| self.store_leaf(old, leaf::encode(&voxels)))
    }
}

#[cfg(test)]
mod tests {
    use crate::tree::{NodeRef, VoxelTree};
    use crate::{MaterialId, VoxelPos};

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
