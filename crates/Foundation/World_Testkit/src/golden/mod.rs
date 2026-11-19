//! Golden files: reference images and text compared with tolerance, updated
//! only on request.
//!
//! A golden file changes only when `UPDATE_GOLDEN=1` is set, which
//! `cargo xtask golden --update` does for you. On a mismatch the actual output
//! and a diff image are written next to the golden file (git-ignored) so the
//! difference can be looked at.

use std::fs;
use std::path::{Path, PathBuf};

pub const UPDATE_ENV: &str = "UPDATE_GOLDEN";

/// Whether this run should rewrite golden files instead of comparing.
pub fn update_requested() -> bool {
    matches!(std::env::var(UPDATE_ENV).as_deref(), Ok("1"))
}

/// An 8-bit RGBA image, row-major, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Rgba8Image {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        assert_eq!(
            data.len(),
            (width as usize) * (height as usize) * 4,
            "RGBA8 data must be width * height * 4 bytes"
        );
        Self {
            width,
            height,
            data,
        }
    }

    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let data = rgba.repeat((width * height) as usize);
        Self::new(width, height, data)
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let img = image::open(path)
            .map_err(|e| format!("cannot read image {}: {e}", path.display()))?
            .to_rgba8();
        Ok(Self::new(img.width(), img.height(), img.into_raw()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let img = image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .ok_or("image buffer does not match its size")?;
        img.save(path)
            .map_err(|e| format!("cannot write image {}: {e}", path.display()))
    }
}

/// How far an image may stray from its golden file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// Largest allowed difference in any single channel, 0 to 255.
    pub per_channel: u8,
    /// Fraction of pixels (0.0 to 1.0) that may exceed `per_channel`.
    pub max_bad_fraction: f32,
}

impl Tolerance {
    /// Every channel of every pixel must match.
    pub const EXACT: Self = Self {
        per_channel: 0,
        max_bad_fraction: 0.0,
    };

    pub const fn per_channel(delta: u8) -> Self {
        Self {
            per_channel: delta,
            max_bad_fraction: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageMismatch {
    SizeDiffers {
        expected: (u32, u32),
        actual: (u32, u32),
    },
    Pixels {
        bad: usize,
        total: usize,
        max_delta: u8,
        /// Bad pixels in red, everything else black.
        diff: Rgba8Image,
    },
}

/// Compares two images under a tolerance.
pub fn compare_images(
    expected: &Rgba8Image,
    actual: &Rgba8Image,
    tolerance: Tolerance,
) -> Result<(), ImageMismatch> {
    if (expected.width, expected.height) != (actual.width, actual.height) {
        return Err(ImageMismatch::SizeDiffers {
            expected: (expected.width, expected.height),
            actual: (actual.width, actual.height),
        });
    }
    let total = (expected.width * expected.height) as usize;
    let mut diff = Vec::with_capacity(total * 4);
    let (mut bad, mut max_delta) = (0usize, 0u8);
    for (e, a) in expected
        .data
        .chunks_exact(4)
        .zip(actual.data.chunks_exact(4))
    {
        let delta = e
            .iter()
            .zip(a)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        max_delta = max_delta.max(delta);
        if delta > tolerance.per_channel {
            bad += 1;
            diff.extend_from_slice(&[255, 0, 0, 255]);
        } else {
            diff.extend_from_slice(&[0, 0, 0, 255]);
        }
    }
    if bad as f32 > tolerance.max_bad_fraction * total as f32 {
        return Err(ImageMismatch::Pixels {
            bad,
            total,
            max_delta,
            diff: Rgba8Image::new(expected.width, expected.height, diff),
        });
    }
    Ok(())
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("golden");
    path.with_file_name(format!("{stem}.{suffix}.png"))
}

/// Asserts that `actual` matches the golden image at `path`.
///
/// With `UPDATE_GOLDEN=1` the golden file is rewritten instead. A missing
/// golden file is a failure, never silently created.
pub fn assert_image_golden(path: &Path, actual: &Rgba8Image, tolerance: Tolerance) {
    if update_requested() {
        actual.save(path).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("updated golden image {}", path.display());
        return;
    }
    if !path.exists() {
        panic!(
            "golden image {} does not exist. Check the output is right, then create it with \
             `cargo xtask golden --update` (or {UPDATE_ENV}=1).",
            path.display()
        );
    }
    let expected = Rgba8Image::load(path).unwrap_or_else(|e| panic!("{e}"));
    match compare_images(&expected, actual, tolerance) {
        Ok(()) => {}
        Err(ImageMismatch::SizeDiffers {
            expected,
            actual: got,
        }) => {
            panic!(
                "golden {} is {expected:?} but the render is {got:?}",
                path.display()
            )
        }
        Err(ImageMismatch::Pixels {
            bad,
            total,
            max_delta,
            diff,
        }) => {
            let (actual_path, diff_path) = (sibling(path, "actual"), sibling(path, "diff"));
            let _ = actual.save(&actual_path);
            let _ = diff.save(&diff_path);
            panic!(
                "{bad} of {total} pixels differ from golden {} (max channel delta {max_delta}, \
                 allowed {}). Wrote {} and {}.",
                path.display(),
                tolerance.per_channel,
                actual_path.display(),
                diff_path.display()
            );
        }
    }
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Asserts that `actual` text matches the golden text file at `path`.
/// Line endings are normalised, so Windows checkouts compare equal.
pub fn assert_text_golden(path: &Path, actual: &str) {
    if update_requested() {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .unwrap_or_else(|e| panic!("cannot create {}: {e}", dir.display()));
        }
        fs::write(path, actual).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        eprintln!("updated golden text {}", path.display());
        return;
    }
    let expected = fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "golden file {} does not exist. Check the output is right, then create it with \
             `cargo xtask golden --update` (or {UPDATE_ENV}=1).",
            path.display()
        )
    });
    let (expected, actual) = (normalize_newlines(&expected), normalize_newlines(actual));
    if expected != actual {
        panic!(
            "text differs from golden {}\n--- expected\n{expected}\n--- actual\n{actual}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("world_testkit_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn identical_images_match_exactly() {
        let a = Rgba8Image::solid(4, 4, [10, 20, 30, 255]);
        assert_eq!(compare_images(&a, &a.clone(), Tolerance::EXACT), Ok(()));
    }

    #[test]
    fn size_difference_is_reported() {
        let (a, b) = (
            Rgba8Image::solid(2, 2, [0; 4]),
            Rgba8Image::solid(3, 2, [0; 4]),
        );
        assert!(matches!(
            compare_images(&a, &b, Tolerance::EXACT),
            Err(ImageMismatch::SizeDiffers { .. })
        ));
    }

    #[test]
    fn small_channel_drift_passes_only_with_tolerance() {
        let a = Rgba8Image::solid(2, 2, [100, 100, 100, 255]);
        let b = Rgba8Image::solid(2, 2, [102, 100, 99, 255]);
        assert!(compare_images(&a, &b, Tolerance::EXACT).is_err());
        assert!(compare_images(&a, &b, Tolerance::per_channel(2)).is_ok());
    }

    #[test]
    fn bad_pixel_fraction_allows_a_few_outliers() {
        let a = Rgba8Image::solid(10, 10, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.data[0] = 255; // one of 100 pixels is badly wrong
        let tolerant = Tolerance {
            per_channel: 0,
            max_bad_fraction: 0.02,
        };
        assert!(compare_images(&a, &b, Tolerance::EXACT).is_err());
        assert!(compare_images(&a, &b, tolerant).is_ok());
    }

    #[test]
    fn diff_image_marks_exactly_the_bad_pixels() {
        let a = Rgba8Image::solid(2, 1, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.data[4] = 200; // second pixel
        match compare_images(&a, &b, Tolerance::EXACT).unwrap_err() {
            ImageMismatch::Pixels {
                bad,
                max_delta,
                diff,
                ..
            } => {
                assert_eq!((bad, max_delta), (1, 200));
                assert_eq!(diff.pixel(0, 0), [0, 0, 0, 255]);
                assert_eq!(diff.pixel(1, 0), [255, 0, 0, 255]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn png_round_trip_is_lossless() {
        let dir = temp_dir("png");
        let mut img = Rgba8Image::solid(3, 2, [1, 2, 3, 255]);
        img.data[5] = 99;
        let path = dir.join("a.png");
        img.save(&path).unwrap();
        assert_eq!(Rgba8Image::load(&path).unwrap(), img);
    }

    #[test]
    fn text_golden_ignores_line_ending_style() {
        let dir = temp_dir("text");
        let path = dir.join("g.txt");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        assert_text_golden(&path, "a\nb\n");
    }

    #[test]
    #[should_panic(expected = "does not exist")]
    fn missing_golden_image_fails_instead_of_being_created() {
        let dir = temp_dir("missing");
        assert_image_golden(
            &dir.join("nope.png"),
            &Rgba8Image::solid(1, 1, [0; 4]),
            Tolerance::EXACT,
        );
    }

    #[test]
    fn mismatch_writes_actual_and_diff_beside_the_golden() {
        let dir = temp_dir("mismatch");
        let golden = dir.join("g.png");
        Rgba8Image::solid(2, 2, [0, 0, 0, 255])
            .save(&golden)
            .unwrap();
        let actual = Rgba8Image::solid(2, 2, [255, 0, 0, 255]);
        let result =
            std::panic::catch_unwind(|| assert_image_golden(&golden, &actual, Tolerance::EXACT));
        assert!(result.is_err());
        assert!(dir.join("g.actual.png").exists() && dir.join("g.diff.png").exists());
    }
}
