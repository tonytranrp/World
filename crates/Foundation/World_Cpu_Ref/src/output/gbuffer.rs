//! The reference G-buffer: one hit record per pixel.

use std::fmt;

use crate::trace::{Face, TraceHit};

/// Per-pixel hits of a frame, row-major from the top row (pixel `(x, y)` is
/// `hits[y * width + x]`), `None` where the ray missed. Plain data with no
/// padding or GPU layout; the GPU's integer hit records compare against the
/// `voxel`, `level`, `face` and `material` fields exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct GBuffer {
    width: u32,
    height: u32,
    hits: Vec<Option<TraceHit>>,
}

/// Where and how two G-buffers differ.
#[derive(Clone, Debug, PartialEq)]
pub enum GBufferMismatch {
    SizeDiffers {
        expected: (u32, u32),
        actual: (u32, u32),
    },
    Pixels {
        /// Pixels whose hits differ.
        count: usize,
        /// The first differing pixel in row-major order, with both hits.
        first: (u32, u32),
        expected: Option<TraceHit>,
        actual: Option<TraceHit>,
    },
}

impl fmt::Display for GBufferMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeDiffers { expected, actual } => {
                write!(f, "G-buffer is {actual:?}, expected {expected:?}")
            }
            Self::Pixels {
                count,
                first,
                expected,
                actual,
            } => write!(
                f,
                "{count} pixels differ; first at {first:?}:\n  expected {expected:?}\n  actual   {actual:?}"
            ),
        }
    }
}

impl GBuffer {
    /// A buffer from row-major hits; `None` if the length is not
    /// `width * height`.
    pub fn new(width: u32, height: u32, hits: Vec<Option<TraceHit>>) -> Option<Self> {
        (hits.len() as u64 == u64::from(width) * u64::from(height)).then_some(Self {
            width,
            height,
            hits,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Every pixel's hit, row-major.
    pub fn hits(&self) -> &[Option<TraceHit>] {
        &self.hits
    }

    /// Pixel `(x, y)`'s hit; `None` for a miss or outside the buffer.
    pub fn hit(&self, x: u32, y: u32) -> Option<&TraceHit> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.hits[(y as usize) * (self.width as usize) + x as usize].as_ref()
    }

    /// Exact comparison: every field of every hit, `t` by bit pattern.
    pub fn compare(&self, actual: &Self) -> Result<(), GBufferMismatch> {
        if (self.width, self.height) != (actual.width, actual.height) {
            return Err(GBufferMismatch::SizeDiffers {
                expected: (self.width, self.height),
                actual: (actual.width, actual.height),
            });
        }
        let mut first = None;
        let mut count = 0;
        for (i, (e, a)) in self.hits.iter().zip(&actual.hits).enumerate() {
            if !same(e, a) {
                count += 1;
                first.get_or_insert(i);
            }
        }
        match first {
            None => Ok(()),
            Some(i) => Err(GBufferMismatch::Pixels {
                count,
                first: (
                    (i % self.width as usize) as u32,
                    (i / self.width as usize) as u32,
                ),
                expected: self.hits[i],
                actual: actual.hits[i],
            }),
        }
    }

    /// A stable 64-bit hash of every hit (FNV-1a over a fixed little-endian
    /// encoding, `t` by bit pattern), for summaries and regression checks.
    pub fn content_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u32(self.width).u32(self.height);
        for hit in &self.hits {
            match hit {
                None => {
                    h.u32(0);
                }
                Some(hit) => {
                    h.u32(1)
                        .u32(hit.voxel.x as u32)
                        .u32(hit.voxel.y as u32)
                        .u32(hit.voxel.z as u32)
                        .u32(hit.level)
                        .u32(face_code(hit.face))
                        .u32(u32::from(hit.material.0))
                        .u32(hit.t.to_bits());
                }
            }
        }
        h.0
    }
}

fn same(a: &Option<TraceHit>, b: &Option<TraceHit>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.voxel == b.voxel
                && a.level == b.level
                && a.face == b.face
                && a.material == b.material
                && a.t.to_bits() == b.t.to_bits()
        }
        _ => false,
    }
}

/// A fixed code per face for hashing.
fn face_code(face: Face) -> u32 {
    match face {
        Face::NegX => 0,
        Face::PosX => 1,
        Face::NegY => 2,
        Face::PosY => 3,
        Face::NegZ => 4,
        Face::PosZ => 5,
        Face::Inside => 6,
    }
}

/// 64-bit FNV-1a.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xCBF2_9CE4_8422_2325)
    }

    fn u32(&mut self, v: u32) -> &mut Self {
        for b in v.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::{MaterialId, VoxelPos};

    fn hit(x: i32) -> TraceHit {
        TraceHit {
            voxel: VoxelPos::new(x, 0, 0),
            level: 0,
            face: Face::NegX,
            material: MaterialId(1),
            t: 2.5,
        }
    }

    #[test]
    fn comparison_reports_count_and_first_pixel() {
        let a = GBuffer::new(2, 2, vec![None, Some(hit(1)), Some(hit(2)), None]).unwrap();
        assert_eq!(a.compare(&a.clone()), Ok(()));
        let mut hits = a.hits().to_vec();
        hits[1] = Some(hit(9));
        hits[3] = Some(hit(3));
        let b = GBuffer::new(2, 2, hits).unwrap();
        match a.compare(&b).unwrap_err() {
            GBufferMismatch::Pixels { count, first, .. } => assert_eq!((count, first), (2, (1, 0))),
            other => panic!("{other:?}"),
        }
        assert_ne!(a.content_hash(), b.content_hash());
        assert!(GBuffer::new(2, 2, vec![None]).is_none());
        assert_eq!(a.hit(1, 0), Some(&hit(1)));
        assert_eq!(a.hit(5, 0), None);
    }

    #[test]
    fn negative_zero_distance_is_a_difference() {
        let mut h = hit(0);
        let a = GBuffer::new(1, 1, vec![Some(h)]).unwrap();
        h.t = -0.0;
        let mut h0 = h;
        h0.t = 0.0;
        let b = GBuffer::new(1, 1, vec![Some(h)]).unwrap();
        let c = GBuffer::new(1, 1, vec![Some(h0)]).unwrap();
        assert!(b.compare(&c).is_err());
        assert!(a.compare(&b).is_err());
    }
}
