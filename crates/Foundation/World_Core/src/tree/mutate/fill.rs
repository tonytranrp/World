//! Box fills.

use super::{Writer, branch, brick};
use crate::consts::TREE_BRANCH_DIM;
use crate::coords::{VoxelBox, VoxelPos, path};
use crate::edit::EditError;
use crate::material::MaterialId;
use crate::tree::node::Child;
use crate::tree::node::leaf::{self, PackedLeaf, Record};

/// A non-empty box inside the world, in world-local coordinates: every `p`
/// with `lo <= p < hi` on each axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalBox {
    lo: [u32; 3],
    hi: [u32; 3],
}

impl LocalBox {
    /// The world-local form of `region`; `None` when it is empty. Fails with
    /// the lowest or highest voxel of the region if that one is outside the
    /// world.
    pub fn of(region: VoxelBox) -> Result<Option<Self>, EditError> {
        if region.is_empty() {
            return Ok(None);
        }
        let (min, end) = (region.min(), region.end());
        let last = VoxelPos::new(end.x - 1, end.y - 1, end.z - 1);
        let lo = path::world_local(min).ok_or(EditError::OutOfWorld { pos: min })?;
        let top = path::world_local(last).ok_or(EditError::OutOfWorld { pos: last })?;
        Ok(Some(Self {
            lo,
            hi: top.map(|v| v + 1),
        }))
    }

    /// The lowest voxel of the box at or above `origin` on every axis: a
    /// voxel of the box inside a cube at `origin` that the box overlaps.
    pub fn lowest_in(&self, origin: [u32; 3]) -> [u32; 3] {
        [0, 1, 2].map(|a| self.lo[a].max(origin[a]))
    }

    /// Whether the box holds the whole cube of `extent` voxels at `origin`.
    fn covers(&self, origin: [u32; 3], extent: u32) -> bool {
        (0..3).all(|a| self.lo[a] <= origin[a] && origin[a] + extent <= self.hi[a])
    }

    /// The child slots, of a node at `origin` with children `span` voxels
    /// wide, that the box overlaps. The box must overlap the node.
    pub fn slots(&self, origin: [u32; 3], span: u32) -> impl Iterator<Item = u32> + use<> {
        let end = TREE_BRANCH_DIM * span;
        let axis = |a: usize| {
            let first = (self.lo[a].max(origin[a]) - origin[a]) / span;
            let last = (self.hi[a].min(origin[a] + end) - 1 - origin[a]) / span;
            first..=last
        };
        let (xs, ys, zs) = (axis(0), axis(1), axis(2));
        zs.flat_map(move |z| {
            let xs = xs.clone();
            ys.clone()
                .flat_map(move |y| xs.clone().map(move |x| x + 4 * y + 16 * z))
        })
    }
}

impl Writer<'_> {
    /// Sets every voxel of `region` inside `child`, a child at `level` whose
    /// lowest voxel is `origin`. Returns the child that replaces it, or
    /// `None` if nothing changed. A child the region covers becomes a filled
    /// child, its subtree freed; only partly covered children are entered.
    pub(super) fn fill(
        &mut self,
        child: Child,
        level: u32,
        origin: [u32; 3],
        region: &LocalBox,
        material: MaterialId,
    ) -> Option<Child> {
        let target = Child::filled(material);
        if child == target {
            return None;
        }
        if region.covers(origin, path::child_span(level + 1)) {
            self.free_subtree(child, level);
            return Some(target);
        }
        if self.at_chunk_boundary(level) {
            return self.in_chunk(child, |w, local| {
                w.fill(local, level, origin, region, material)
            });
        }
        let existing = self.existing(child);
        let span = path::child_span(level);
        let child_origin = |slot: u32| {
            let offset = path::slot_offset(slot);
            [0, 1, 2].map(|a| origin[a] + offset[a] * span)
        };

        if level == 1 {
            let mut records = self.records(child);
            let mut changed = false;
            for slot in region.slots(origin, span) {
                let at = child_origin(slot);
                let old = records[slot as usize];
                let new = if region.covers(at, span) {
                    let filled = Record::filled(material);
                    if old == filled {
                        continue;
                    }
                    PackedLeaf::Inline(filled)
                } else {
                    let mut voxels = self.leaf_voxels(old);
                    let before = voxels;
                    for voxel in region.slots(at, 1) {
                        voxels[voxel as usize] = material;
                    }
                    if voxels == before {
                        continue;
                    }
                    leaf::encode(&voxels)
                };
                records[slot as usize] = self.store_leaf(old, new);
                changed = true;
            }
            return changed.then(|| self.write(existing, 1, &brick::pack(&records)));
        }

        let mut children = self.children(child);
        let mut changed = false;
        for slot in region.slots(origin, span) {
            let at = child_origin(slot);
            if let Some(new) = self.fill(children[slot as usize], level - 1, at, region, material) {
                children[slot as usize] = new;
                changed = true;
            }
        }
        changed.then(|| self.write(existing, level, &branch::pack(&children)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::{WORLD_END_VOXEL, WORLD_MIN_VOXEL};

    fn local(min: [i32; 3], end: [i32; 3]) -> LocalBox {
        LocalBox::of(VoxelBox::new(min.into(), end.into()))
            .unwrap()
            .unwrap()
    }

    #[test]
    fn empty_regions_are_none_and_outside_corners_are_errors() {
        let empty = VoxelBox::new(VoxelPos::new(0, 0, 0), VoxelPos::new(0, 5, 5));
        assert_eq!(LocalBox::of(empty), Ok(None));

        let low = VoxelBox::new(
            VoxelPos::new(WORLD_MIN_VOXEL - 1, 0, 0),
            VoxelPos::new(0, 1, 1),
        );
        assert_eq!(
            LocalBox::of(low),
            Err(EditError::OutOfWorld {
                pos: VoxelPos::new(WORLD_MIN_VOXEL - 1, 0, 0)
            })
        );
        let high = VoxelBox::new(
            VoxelPos::new(0, 0, 0),
            VoxelPos::new(1, WORLD_END_VOXEL + 1, 1),
        );
        assert_eq!(
            LocalBox::of(high),
            Err(EditError::OutOfWorld {
                pos: VoxelPos::new(0, WORLD_END_VOXEL, 0)
            })
        );
        assert!(LocalBox::of(VoxelBox::WORLD).unwrap().is_some());
    }

    #[test]
    fn covers_needs_the_whole_cube() {
        let o = (-WORLD_MIN_VOXEL) as u32;
        let b = local([0, 0, 0], [16, 16, 17]);
        assert!(b.covers([o, o, o], 16));
        assert!(b.covers([o + 4, o + 8, o + 12], 4));
        assert!(!b.covers([o, o, o], 64));
        assert!(!b.covers([o - 1, o, o], 4));
    }

    #[test]
    fn slots_are_exactly_the_overlapped_children() {
        let o = (-WORLD_MIN_VOXEL) as u32;
        // Voxels 3..5 on x, 0..1 on y and z: children 0 and 1 of a 4-wide span.
        let b = local([3, 0, 0], [5, 1, 1]);
        assert_eq!(b.slots([o, o, o], 4).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(b.slots([o, o, o], 1).collect::<Vec<_>>(), [3]);
        let all = local([-8, -8, -8], [8, 8, 8]);
        assert_eq!(all.slots([o - 8, o - 8, o - 8], 4).count(), 64);
        assert_eq!(all.slots([o - 32, o - 32, o - 32], 16).count(), 8);
    }
}
