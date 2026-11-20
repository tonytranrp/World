//! The pinhole camera and per-pixel primary rays.
//!
//! Conventions (stated once here; the rest of the crate refers to them):
//!
//! - Units are world voxels. [`CameraDesc::from_metres`] converts a position
//!   in metres by multiplying by `VOXELS_PER_METER` (64, exact).
//! - Right-handed axes with `+Y` up. The camera looks along `forward`;
//!   `right = normalize(cross(forward, up))` and the true up is
//!   `cross(right, forward)`, so with `forward = -Z` and `up = +Y`, right is
//!   `+X`.
//! - Pixel `(px, py)` counts from the top-left; rows go down. A ray passes
//!   through the pixel centre `(px + 0.5, py + 0.5)`.
//! - The field of view is given as `tan(vertical_fov / 2)`, so no platform
//!   `tan` is ever called and the GPU receives the very same constant.
//!
//! Order of operations for pixel `(px, py)` in a `w x h` image, all `f32`:
//!
//! ```text
//! ndc_x = (px + 0.5) / w * 2 - 1
//! ndc_y = 1 - (py + 0.5) / h * 2
//! u = ndc_x * half_width        where half_width = tan_half_fov_y * (w / h)
//! v = ndc_y * tan_half_fov_y
//! d = forward + right * u + up * v    (per axis: (f + r * u) + up * v)
//! dir = d / sqrt(d.d)                 (d.d = (x * x + y * y) + z * z)
//! ```
//!
//! `sqrt` and division are correctly rounded IEEE operations, so the rays are
//! the same on every machine.

mod ray;

pub use ray::{Ray, RayError};

use std::fmt;

use world_core::consts::VOXELS_PER_METER;

/// What a camera is, before validation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraDesc {
    /// The eye position, in voxels.
    pub eye: [f32; 3],
    /// The viewing direction; any non-zero length.
    pub forward: [f32; 3],
    /// Roughly up; must not be parallel to `forward`.
    pub up: [f32; 3],
    /// `tan(vertical_fov / 2)`, above 0 (1.0 is a 90 degree field of view).
    pub tan_half_fov_y: f32,
    /// Internal render resolution in pixels.
    pub width: u32,
    pub height: u32,
}

impl CameraDesc {
    /// The same camera with `eye` given in metres.
    pub fn from_metres(mut self) -> Self {
        let vpm = VOXELS_PER_METER as f32;
        self.eye = self.eye.map(|c| c * vpm);
        self
    }
}

/// Why a camera description was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraError {
    /// A value is NaN or infinite.
    NonFinite,
    /// Width or height is zero, or the image has more than 2^24 pixels on an
    /// axis (pixel centres would no longer be exact in `f32`).
    BadResolution,
    /// The field of view is not in `(0, 1000]` as `tan(fov / 2)`.
    BadFieldOfView,
    /// `forward` is zero, or parallel to `up`.
    DegenerateBasis,
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NonFinite => "a camera value is NaN or infinite",
            Self::BadResolution => "the resolution must be 1 to 2^24 pixels on each axis",
            Self::BadFieldOfView => "tan(vertical_fov / 2) must be above 0 and at most 1000",
            Self::DegenerateBasis => "forward must be non-zero and not parallel to up",
        })
    }
}

impl std::error::Error for CameraError {}

/// A validated pinhole camera with its orthonormal basis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    desc: CameraDesc,
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    half_width: f32,
}

impl Camera {
    pub fn new(desc: CameraDesc) -> Result<Self, CameraError> {
        let values = desc
            .eye
            .iter()
            .chain(&desc.forward)
            .chain(&desc.up)
            .chain([&desc.tan_half_fov_y]);
        if !values.into_iter().all(|v| v.is_finite()) {
            return Err(CameraError::NonFinite);
        }
        const MAX_SIDE: u32 = 1 << 24;
        if !(1..=MAX_SIDE).contains(&desc.width) || !(1..=MAX_SIDE).contains(&desc.height) {
            return Err(CameraError::BadResolution);
        }
        if !(desc.tan_half_fov_y > 0.0 && desc.tan_half_fov_y <= 1000.0) {
            return Err(CameraError::BadFieldOfView);
        }
        let forward = normalize(desc.forward).ok_or(CameraError::DegenerateBasis)?;
        let right = normalize(cross(forward, desc.up)).ok_or(CameraError::DegenerateBasis)?;
        let up = cross(right, forward);
        let half_width = desc.tan_half_fov_y * (desc.width as f32 / desc.height as f32);
        Ok(Self {
            desc,
            forward,
            right,
            up,
            half_width,
        })
    }

    pub fn desc(&self) -> &CameraDesc {
        &self.desc
    }

    pub fn width(&self) -> u32 {
        self.desc.width
    }

    pub fn height(&self) -> u32 {
        self.desc.height
    }

    /// The unit viewing direction, right and up vectors.
    pub fn basis(&self) -> [[f32; 3]; 3] {
        [self.forward, self.right, self.up]
    }

    /// The primary ray through the centre of pixel `(px, py)`, in the order
    /// of operations documented on the module; `None` outside the image.
    pub fn ray(&self, px: u32, py: u32) -> Option<Ray> {
        if px >= self.desc.width || py >= self.desc.height {
            return None;
        }
        let (w, h) = (self.desc.width as f32, self.desc.height as f32);
        let ndc_x = (px as f32 + 0.5) / w * 2.0 - 1.0;
        let ndc_y = 1.0 - (py as f32 + 0.5) / h * 2.0;
        let u = ndc_x * self.half_width;
        let v = ndc_y * self.desc.tan_half_fov_y;
        let (f, r, up) = (self.forward, self.right, self.up);
        let d = [
            (f[0] + r[0] * u) + up[0] * v,
            (f[1] + r[1] * u) + up[1] * v,
            (f[2] + r[2] * u) + up[2] * v,
        ];
        // `forward` is a unit vector orthogonal to `right` and `up`, so `d`
        // has length at least 1 and normalizes.
        let dir = normalize(d)?;
        Some(Ray::new(self.desc.eye, dir))
    }
}

/// `a x b`.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `v / sqrt((x * x + y * y) + z * z)`, or `None` for a zero or overflowing
/// length.
fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (len > 0.0 && len.is_finite()).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc() -> CameraDesc {
        CameraDesc {
            eye: [1.0, 2.0, 3.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            tan_half_fov_y: 1.0,
            width: 4,
            height: 2,
        }
    }

    #[test]
    fn the_basis_is_right_handed_with_y_up() {
        let c = Camera::new(desc()).unwrap();
        assert_eq!(
            c.basis(),
            [[0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
    }

    #[test]
    fn pixel_centres_map_to_known_directions() {
        let c = Camera::new(desc()).unwrap();
        // Pixel (0, 0) is top-left: u = -0.75 * 2, v = +0.5.
        let r = c.ray(0, 0).unwrap();
        let d = [-1.5f32, 0.5, -1.0];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert_eq!(r.dir, [d[0] / len, d[1] / len, d[2] / len]);
        assert_eq!(r.origin, [1.0, 2.0, 3.0]);
        // Symmetric pixels mirror each other.
        let (a, b) = (c.ray(1, 0).unwrap(), c.ray(2, 1).unwrap());
        assert_eq!((a.dir[0], a.dir[1]), (-b.dir[0], -b.dir[1]));
        assert_eq!(c.ray(4, 0), None);
        assert_eq!(c.ray(0, 2), None);
    }

    #[test]
    fn degenerate_cameras_are_rejected() {
        let bad = |f: &dyn Fn(&mut CameraDesc)| {
            let mut d = desc();
            f(&mut d);
            Camera::new(d).unwrap_err()
        };
        assert_eq!(bad(&|d| d.eye[0] = f32::NAN), CameraError::NonFinite);
        assert_eq!(bad(&|d| d.width = 0), CameraError::BadResolution);
        assert_eq!(
            bad(&|d| d.tan_half_fov_y = 0.0),
            CameraError::BadFieldOfView
        );
        assert_eq!(bad(&|d| d.forward = [0.0; 3]), CameraError::DegenerateBasis);
        assert_eq!(
            bad(&|d| d.up = [0.0, 0.0, 2.0]),
            CameraError::DegenerateBasis
        );
    }

    #[test]
    fn metres_convert_to_voxels() {
        assert_eq!(desc().from_metres().eye, [64.0, 128.0, 192.0]);
    }
}
