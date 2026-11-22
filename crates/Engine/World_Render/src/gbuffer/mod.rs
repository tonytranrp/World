//! The G-buffer, version 0: per traced pixel, the hit in two planes.
//!
//! # Camera traces: two planes of 2 words (16 bytes per pixel)
//!
//! A camera trace (`MODE_CAMERA`) of `n` pixels writes words `0..2n` (the
//! shading plane) and `2n..4n` (the ray plane), row-major, 2 words per pixel
//! in each:
//!
//! | plane | word | content |
//! |-------|------|---------|
//! | shading | 0 | `material \| level << 16 \| face << 20 \| flags << 24` |
//! | shading | 1 | depth, f32 bits, as Bevy's depth texture stores it (reverse Z) |
//! | ray | 0 | `t`, f32 bits: distance along the unit ray direction, in voxels |
//! | ray | 1 | traversal iterations (debug) |
//!
//! The shading pass reads only the shading plane (8 bytes a pixel); the ray
//! plane is for the sampled ray statistics and later lighting.
//!
//! # Full records: 8 words (32 bytes) per pixel, row-major
//!
//! Rays read from the buffer (`MODE_RAYS`, tests) and camera traces in
//! `MODE_CAMERA_FULL` (tests) write one full record per pixel:
//!
//! | word | content |
//! |------|---------|
//! | 0..3 | the hit voxel `x, y, z` (i32 bits, world voxel coordinates): the first voxel of the hit cell the ray entered |
//! | 3 | `material \| level << 16 \| face << 20 \| flags << 24` |
//! | 4 | `t`, f32 bits |
//! | 5 | depth, f32 bits |
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
//! In `MODE_RAYS` the pass reads its ray from the same record (origin in
//! words 0..3, direction in 3..6, f32 bits) before overwriting it, so CPU and
//! GPU trace bit-identical rays.
//!
//! Normals, motion vectors and the split into textures come with lighting.

use world_core::{MaterialId, VoxelPos};

/// Words per pixel in each plane of a camera trace.
pub const HIT_PLANE_WORDS: u32 = 2;
/// Words per pixel of a camera trace's buffer: both planes.
pub const HIT_WORDS: u32 = 2 * HIT_PLANE_WORDS;
/// Words per pixel of a full record (`MODE_RAYS`, `MODE_CAMERA_FULL`).
pub const HIT_RECORD_WORDS: u32 = 8;
pub const HIT_FLAG_HIT: u32 = 1;
pub const HIT_FLAG_RESIDENCY: u32 = 2;
pub const HIT_FLAG_FOOTPRINT: u32 = 4;
pub const HIT_FLAG_EXHAUSTED: u32 = 8;
pub const FACE_INSIDE: u32 = 6;

/// The hit record constants, generated into the shaders by `cargo xtask codegen`.
pub const SHADER_CONSTANTS: &[(&str, u32)] = &[
    ("HIT_PLANE_WORDS", HIT_PLANE_WORDS),
    ("HIT_RECORD_WORDS", HIT_RECORD_WORDS),
    ("HIT_FLAG_HIT", HIT_FLAG_HIT),
    ("HIT_FLAG_RESIDENCY", HIT_FLAG_RESIDENCY),
    ("HIT_FLAG_FOOTPRINT", HIT_FLAG_FOOTPRINT),
    ("HIT_FLAG_EXHAUSTED", HIT_FLAG_EXHAUSTED),
    ("FACE_INSIDE", FACE_INSIDE),
];

/// Bytes of a camera trace's G-buffer for `pixels` pixels.
pub const fn camera_buffer_bytes(pixels: u64) -> u64 {
    pixels * HIT_WORDS as u64 * 4
}

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

    /// Decodes full record `words` (at least [`HIT_RECORD_WORDS`] long);
    /// `None` if short.
    pub fn decode(words: &[u32]) -> Option<Self> {
        let w = words.get(..HIT_RECORD_WORDS as usize)?;
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

/// The packed word, depth, `t` and steps of every pixel of a camera trace's
/// buffer (`words`, both planes), in pixel order.
pub fn camera_records(words: &[u32]) -> impl Iterator<Item = [u32; 4]> + '_ {
    let n = words.len() / HIT_WORDS as usize;
    let plane = HIT_PLANE_WORDS as usize;
    let (shading, ray) = words[..n * HIT_WORDS as usize].split_at(n * plane);
    shading
        .chunks_exact(plane)
        .zip(ray.chunks_exact(plane))
        .map(|(s, r)| [s[0], s[1], r[0], r[1]])
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

    #[test]
    fn camera_records_pair_the_two_planes() {
        // Two pixels: shading plane, then ray plane.
        let words = [1, 2, 3, 4, 10, 20, 30, 40];
        let r: Vec<_> = camera_records(&words).collect();
        assert_eq!(r, vec![[1, 2, 10, 20], [3, 4, 30, 40]]);
        assert_eq!(camera_buffer_bytes(2), 32);
    }
}
