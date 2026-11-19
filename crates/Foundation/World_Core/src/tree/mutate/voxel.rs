//! Single-voxel writes.

use super::{Writer, branch};
use crate::coords::path;
use crate::material::MaterialId;
use crate::tree::node::{Child, NodeKind, leaf};

impl Writer<'_> {
    /// Sets the voxel at world-local `local` inside `child`, a child at
    /// `level`. Returns the child that replaces it, or `None` if the voxel
    /// already holds `material`.
    pub(super) fn set_voxel(
        &mut self,
        child: Child,
        level: u32,
        local: [u32; 3],
        material: MaterialId,
    ) -> Option<Child> {
        if child == Child::filled(material) {
            return None;
        }
        let slot = path::child_slot(local, level);
        let existing = self.existing(child);

        if level == 0 {
            let mut voxels = self.leaf_voxels(child);
            if voxels[slot as usize] == material {
                return None;
            }
            voxels[slot as usize] = material;
            return Some(self.write(existing, NodeKind::Leaf, leaf::encode(&voxels)));
        }

        // Air or uniform: the target child is the same fill, which the
        // recursion splits in turn.
        let old = match existing {
            Some((_, node)) => self.child(node, slot),
            None => child,
        };
        let new = self.set_voxel(old, level - 1, local, material)?;
        if new == old {
            // The child node was rebuilt in place: this node's words stand.
            return Some(child);
        }
        let mut children = self.children(child);
        children[slot as usize] = new;
        Some(self.write(existing, NodeKind::Branch, branch::pack(&children)))
    }
}

#[cfg(test)]
mod tests {
    use crate::tree::{NodeRef, TreeRead, VoxelTree};
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
        assert_eq!(s.words, 9);
    }
}
