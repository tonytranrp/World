use std::fmt;

use world_core::{Edit, MaterialId, VoxelPos};

use super::world::{Bounds, VoxelModel};

/// The first voxel where two models disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divergence {
    pub pos: VoxelPos,
    pub left: MaterialId,
    pub right: MaterialId,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "models diverge at {:?}: left has material {}, right has material {}",
            self.pos, self.left.0, self.right.0
        )
    }
}

/// Compares two models over every voxel of `bounds`.
pub fn assert_models_agree(
    left: &impl VoxelModel,
    right: &impl VoxelModel,
    bounds: Bounds,
) -> Result<(), Divergence> {
    let lo = bounds.min;
    for z in 0..bounds.size[2] as i32 {
        for y in 0..bounds.size[1] as i32 {
            for x in 0..bounds.size[0] as i32 {
                let pos = VoxelPos::new(lo.x + x, lo.y + y, lo.z + z);
                let (l, r) = (left.get(pos), right.get(pos));
                if l != r {
                    return Err(Divergence {
                        pos,
                        left: l,
                        right: r,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Applies the same edits to both models, then compares them. This is the
/// check the 64-tree must pass against the dense model for any edit sequence.
pub fn compare_after_edits(
    left: &mut impl VoxelModel,
    right: &mut impl VoxelModel,
    edits: &[Edit],
    bounds: Bounds,
) -> Result<(), Divergence> {
    for &edit in edits {
        left.apply(edit);
        right.apply(edit);
    }
    assert_models_agree(left, right, bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dense::DenseWorld;

    fn bounds() -> Bounds {
        Bounds::new(VoxelPos::new(0, 0, 0), [3, 3, 3])
    }

    #[test]
    fn equal_models_agree() {
        let (mut a, mut b) = (DenseWorld::new(bounds()), DenseWorld::new(bounds()));
        let edits = [Edit::new(VoxelPos::new(1, 1, 1), MaterialId(2))];
        assert_eq!(
            compare_after_edits(&mut a, &mut b, &edits, bounds()),
            Ok(())
        );
    }

    #[test]
    fn a_single_different_voxel_is_found() {
        let (mut a, b) = (DenseWorld::new(bounds()), DenseWorld::new(bounds()));
        a.set(VoxelPos::new(2, 0, 1), MaterialId(7));
        assert_eq!(
            assert_models_agree(&a, &b, bounds()),
            Err(Divergence {
                pos: VoxelPos::new(2, 0, 1),
                left: MaterialId(7),
                right: MaterialId::AIR,
            })
        );
    }

    #[test]
    fn divergence_message_names_the_position() {
        let d = Divergence {
            pos: VoxelPos::new(1, 2, 3),
            left: MaterialId(1),
            right: MaterialId(2),
        };
        assert!(d.to_string().contains("x: 1"));
    }
}
