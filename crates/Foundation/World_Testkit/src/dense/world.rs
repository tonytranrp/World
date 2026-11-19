use world_core::{Edit, MaterialId, VoxelBox, VoxelPos};

use crate::hash::{HashState, StateHasher};

/// A box of voxels: `min` is the lowest voxel, `size` the extent per axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub min: VoxelPos,
    pub size: [u32; 3],
}

impl Bounds {
    pub const fn new(min: VoxelPos, size: [u32; 3]) -> Self {
        Self { min, size }
    }

    pub fn contains(&self, pos: VoxelPos) -> bool {
        let p = pos.to_array();
        let lo = self.min.to_array();
        (0..3).all(|a| p[a] >= lo[a] && (p[a] as i64) < lo[a] as i64 + self.size[a] as i64)
    }

    pub fn volume(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    /// The same voxels as a World_Core box.
    pub const fn as_box(&self) -> VoxelBox {
        VoxelBox::from_min_size(self.min, self.size)
    }
}

/// One step of a random edit sequence: a single voxel or a whole box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOp {
    Set(Edit),
    Fill {
        region: VoxelBox,
        material: MaterialId,
    },
}

/// What the dense model and the 64-tree both promise, so the same comparison
/// code checks both.
pub trait VoxelModel {
    /// The material at `pos`; air anywhere nothing has been set.
    fn get(&self, pos: VoxelPos) -> MaterialId;

    /// Sets one voxel. Setting air removes the voxel.
    fn set(&mut self, pos: VoxelPos, material: MaterialId);

    fn apply(&mut self, edit: Edit) {
        self.set(edit.pos, edit.material);
    }

    /// Sets every voxel of `region`, one at a time. An empty box does nothing.
    fn fill_box(&mut self, region: VoxelBox, material: MaterialId) {
        let (lo, end) = (region.min(), region.end());
        for z in lo.z..end.z {
            for y in lo.y..end.y {
                for x in lo.x..end.x {
                    self.set(VoxelPos::new(x, y, z), material);
                }
            }
        }
    }

    fn apply_op(&mut self, op: EditOp) {
        match op {
            EditOp::Set(edit) => self.apply(edit),
            EditOp::Fill { region, material } => self.fill_box(region, material),
        }
    }
}

/// A fixed-size dense grid. Reads outside the bounds return air; writes outside
/// the bounds panic, because a test that does so has a bug.
#[derive(Clone, Debug)]
pub struct DenseWorld {
    bounds: Bounds,
    cells: Vec<MaterialId>,
}

impl DenseWorld {
    pub fn new(bounds: Bounds) -> Self {
        Self {
            bounds,
            cells: vec![MaterialId::AIR; bounds.volume()],
        }
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    fn index(&self, pos: VoxelPos) -> Option<usize> {
        if !self.bounds.contains(pos) {
            return None;
        }
        let p = pos.to_array();
        let lo = self.bounds.min.to_array();
        let [sx, sy, _] = self.bounds.size.map(|s| s as usize);
        let (x, y, z) = (
            (p[0] - lo[0]) as usize,
            (p[1] - lo[1]) as usize,
            (p[2] - lo[2]) as usize,
        );
        Some(x + sx * (y + sy * z))
    }

    /// Number of non-air voxels.
    pub fn solid_count(&self) -> usize {
        self.cells.iter().filter(|m| !m.is_air()).count()
    }

    /// Every non-air voxel with its position, in a fixed order.
    pub fn solids(&self) -> impl Iterator<Item = (VoxelPos, MaterialId)> + '_ {
        let lo = self.bounds.min;
        let [sx, sy, _] = self.bounds.size.map(|s| s as usize);
        self.cells.iter().enumerate().filter_map(move |(i, &m)| {
            if m.is_air() {
                return None;
            }
            let (x, y, z) = (i % sx, (i / sx) % sy, i / (sx * sy));
            Some((
                VoxelPos::new(lo.x + x as i32, lo.y + y as i32, lo.z + z as i32),
                m,
            ))
        })
    }
}

impl VoxelModel for DenseWorld {
    fn get(&self, pos: VoxelPos) -> MaterialId {
        self.index(pos).map_or(MaterialId::AIR, |i| self.cells[i])
    }

    fn set(&mut self, pos: VoxelPos, material: MaterialId) {
        let i = self.index(pos).unwrap_or_else(|| {
            panic!(
                "edit at {pos:?} is outside the dense world {:?}",
                self.bounds
            )
        });
        self.cells[i] = material;
    }
}

impl HashState for DenseWorld {
    fn hash_state(&self, hasher: &mut StateHasher) {
        for v in self.bounds.min.to_array() {
            hasher.write_i32(v);
        }
        for s in self.bounds.size {
            hasher.write_u32(s);
        }
        for m in &self.cells {
            hasher.write_u16(m.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> DenseWorld {
        DenseWorld::new(Bounds::new(VoxelPos::new(-2, 0, 5), [4, 3, 2]))
    }

    #[test]
    fn new_world_is_all_air() {
        let w = world();
        assert_eq!(w.solid_count(), 0);
        assert_eq!(w.get(VoxelPos::new(-2, 0, 5)), MaterialId::AIR);
    }

    #[test]
    fn set_then_get_round_trips_at_every_corner() {
        let mut w = world();
        let corners = [
            VoxelPos::new(-2, 0, 5),
            VoxelPos::new(1, 0, 5),
            VoxelPos::new(-2, 2, 5),
            VoxelPos::new(-2, 0, 6),
            VoxelPos::new(1, 2, 6),
        ];
        for (i, c) in corners.into_iter().enumerate() {
            w.set(c, MaterialId(i as u16 + 1));
        }
        for (i, c) in corners.into_iter().enumerate() {
            assert_eq!(w.get(c), MaterialId(i as u16 + 1));
        }
        assert_eq!(w.solid_count(), 5);
    }

    #[test]
    fn reads_outside_are_air() {
        let w = world();
        assert_eq!(w.get(VoxelPos::new(100, 0, 0)), MaterialId::AIR);
        assert_eq!(w.get(VoxelPos::new(-3, 0, 5)), MaterialId::AIR);
    }

    #[test]
    #[should_panic(expected = "outside the dense world")]
    fn writes_outside_panic() {
        world().set(VoxelPos::new(2, 0, 5), MaterialId(1));
    }

    #[test]
    fn setting_air_removes_a_voxel() {
        let mut w = world();
        let p = VoxelPos::new(0, 1, 5);
        w.set(p, MaterialId(3));
        w.set(p, MaterialId::AIR);
        assert_eq!(w.solid_count(), 0);
    }

    #[test]
    fn solids_lists_positions_that_read_back() {
        let mut w = world();
        w.set(VoxelPos::new(-1, 2, 6), MaterialId(4));
        w.set(VoxelPos::new(1, 0, 5), MaterialId(2));
        let solids: Vec<_> = w.solids().collect();
        assert_eq!(solids.len(), 2);
        for (pos, material) in solids {
            assert_eq!(w.get(pos), material);
        }
    }

    #[test]
    fn fill_box_sets_exactly_the_box() {
        let mut w = world();
        let region = VoxelBox::new(VoxelPos::new(-1, 1, 5), VoxelPos::new(1, 3, 6));
        w.fill_box(region, MaterialId(2));
        assert_eq!(w.solid_count(), 4);
        for (pos, material) in w.solids() {
            assert!(region.contains(pos));
            assert_eq!(material, MaterialId(2));
        }
    }

    #[test]
    fn empty_fill_does_nothing() {
        let mut w = world();
        let far = VoxelPos::new(1_000, 0, 0);
        w.apply_op(EditOp::Fill {
            region: VoxelBox::new(far, far),
            material: MaterialId(1),
        });
        assert_eq!(w.solid_count(), 0);
    }

    #[test]
    fn state_hash_changes_with_content() {
        let mut w = world();
        let before = w.state_hash();
        w.set(VoxelPos::new(0, 0, 5), MaterialId(1));
        assert_ne!(before, w.state_hash());
    }
}
