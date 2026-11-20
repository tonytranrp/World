//! A ray in voxel units.

use std::fmt;

/// A ray in world voxel units: the voxel at `(x, y, z)` occupies
/// `[x, x + 1) x [y, y + 1) x [z, z + 1)`.
///
/// Distances along it are in units of the direction's length, so a
/// normalized direction measures `t` in voxels. There is deliberately no
/// stored inverse direction: the tracer computes every distance as
/// `(boundary - origin) / dir`, a division, which is what makes it
/// bit-identical to the dense DDA reference (multiplying by a rounded inverse
/// would not be).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
}

/// Why a ray cannot be traced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayError {
    /// A component of the origin or direction is NaN or infinite.
    NonFinite,
    /// The direction is zero on every axis.
    ZeroDirection,
}

impl fmt::Display for RayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => f.write_str("the ray has a NaN or infinite component"),
            Self::ZeroDirection => f.write_str("the ray direction is zero"),
        }
    }
}

impl std::error::Error for RayError {}

impl Ray {
    pub const fn new(origin: [f32; 3], dir: [f32; 3]) -> Self {
        Self { origin, dir }
    }

    /// Whether the ray can be traced: finite, with a non-zero direction.
    pub fn check(&self) -> Result<(), RayError> {
        if !self.origin.iter().chain(&self.dir).all(|c| c.is_finite()) {
            return Err(RayError::NonFinite);
        }
        if self.dir.iter().all(|&c| c == 0.0) {
            return Err(RayError::ZeroDirection);
        }
        Ok(())
    }

    /// The point at distance `t`: `origin + dir * t` per axis.
    pub fn at(&self, t: f32) -> [f32; 3] {
        [
            self.origin[0] + self.dir[0] * t,
            self.origin[1] + self.dir[1] * t,
            self.origin[2] + self.dir[2] * t,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degenerate_rays_are_rejected() {
        assert_eq!(
            Ray::new([0.0; 3], [0.0, -0.0, 0.0]).check(),
            Err(RayError::ZeroDirection)
        );
        assert_eq!(
            Ray::new([f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]).check(),
            Err(RayError::NonFinite)
        );
        assert_eq!(
            Ray::new([0.0; 3], [f32::INFINITY, 0.0, 0.0]).check(),
            Err(RayError::NonFinite)
        );
        assert_eq!(Ray::new([1.0; 3], [0.0, 0.0, -1.0]).check(), Ok(()));
        assert_eq!(
            Ray::new([1.0; 3], [0.0, 0.0, -1.0]).at(2.0),
            [1.0, 1.0, -1.0]
        );
    }
}
