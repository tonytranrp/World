//! The G-buffer, version 0: one integer hit record per pixel.
//!
//! # Hit record, 8 words (32 bytes) per pixel, row-major
//!
//! | word | content |
//! |------|---------|
//! | 0..3 | the hit voxel `x, y, z` (i32 bits, world voxel coordinates): the first voxel of the hit cell the ray entered |
//! | 3 | `material \| level << 16 \| face << 20 \| flags << 24` |
//! | 4 | `t`, f32 bits: distance along the unit ray direction, in voxels |
//! | 5 | depth, f32 bits, as Bevy's depth texture stores it (reverse Z) |
//! | 6 | traversal iterations (debug) |
//! | 7 | reserved |
//!
//! `level` is the cell size the hit was decided at: the cell is `4^level`
//! voxels per axis and contains the hit voxel (0 for an exact voxel). `face`
//! is the entry face (0 -x, 1 +x, 2 -y, 3 +y, 4 -z, 5 +z, 6 inside). Flags:
//! [`HIT_FLAG_HIT`], [`HIT_FLAG_RESIDENCY`] when the level came from a
//! not-resident node, [`HIT_FLAG_FOOTPRINT`] when it came from the pixel
//! footprint, [`HIT_FLAG_EXHAUSTED`] when the iteration cap ended the ray.
//!
//! In test mode the pass reads its ray from the same record (origin in words
//! 0..3, direction in 3..6, f32 bits) before overwriting it, so CPU and GPU
//! trace bit-identical rays.
//!
//! Normals, motion vectors and the split into textures come with lighting.

use world_core::{MaterialId, VoxelPos};

pub const HIT_WORDS: u32 = 8;
pub const HIT_FLAG_HIT: u32 = 1;
pub const HIT_FLAG_RESIDENCY: u32 = 2;
pub const HIT_FLAG_FOOTPRINT: u32 = 4;
pub const HIT_FLAG_EXHAUSTED: u32 = 8;
pub const FACE_INSIDE: u32 = 6;

/// The hit record constants, generated into the shaders by `cargo xtask codegen`.
pub const SHADER_CONSTANTS: &[(&str, u32)] = &[
    ("HIT_WORDS", HIT_WORDS),
    ("HIT_FLAG_HIT", HIT_FLAG_HIT),
    ("HIT_FLAG_RESIDENCY", HIT_FLAG_RESIDENCY),
    ("HIT_FLAG_FOOTPRINT", HIT_FLAG_FOOTPRINT),
    ("HIT_FLAG_EXHAUSTED", HIT_FLAG_EXHAUSTED),
    ("FACE_INSIDE", FACE_INSIDE),
];

/// One decoded hit record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuHit {
    pub voxel: VoxelPos,
    pub material: MaterialId,
    pub level: u32,
    pub face: u32,
    pub flags: u32,
    pub t: f32,
    pub depth: f32,
    pub steps: u32,
}

impl GpuHit {
    pub fn is_hit(&self) -> bool {
        self.flags & HIT_FLAG_HIT != 0
    }

    /// Decodes record `words` (at least [`HIT_WORDS`] long); `None` if short.
    pub fn decode(words: &[u32]) -> Option<Self> {
        let w = words.get(..HIT_WORDS as usize)?;
        Some(Self {
            voxel: VoxelPos::new(w[0] as i32, w[1] as i32, w[2] as i32),
            material: MaterialId(w[3] as u16),
            level: (w[3] >> 16) & 0xF,
            face: (w[3] >> 20) & 0xF,
            flags: w[3] >> 24,
            t: f32::from_bits(w[4]),
            depth: f32::from_bits(w[5]),
            steps: w[6],
        })
    }
}

/// Writes rays into a hit buffer for test mode.
pub fn encode_rays(rays: impl IntoIterator<Item = ([f32; 3], [f32; 3])>) -> Vec<u32> {
    let mut out = Vec::new();
    for (o, d) in rays {
        out.extend(o.iter().chain(d.iter()).map(|v| v.to_bits()));
        out.extend([0, 0]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_decode_every_field() {
        let w = [
            (-5i32) as u32,
            7,
            9,
            12 | 1 << 16 | 3 << 20 | (HIT_FLAG_HIT | HIT_FLAG_RESIDENCY) << 24,
            2.5f32.to_bits(),
            0.25f32.to_bits(),
            40,
            0,
        ];
        let h = GpuHit::decode(&w).unwrap();
        assert_eq!(h.voxel, VoxelPos::new(-5, 7, 9));
        assert_eq!((h.material, h.level, h.face), (MaterialId(12), 1, 3));
        assert!(h.is_hit() && h.flags & HIT_FLAG_RESIDENCY != 0);
        assert_eq!((h.t, h.depth, h.steps), (2.5, 0.25, 40));
        assert!(GpuHit::decode(&w[..4]).is_none());
    }
}
