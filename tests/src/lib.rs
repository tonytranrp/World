//! Shared helpers for the cross-crate suites: where fixtures live and how to load them.

use std::fs;
use std::path::PathBuf;

use world_core::{MaterialId, VoxelPos};
use world_testkit::dense::{Bounds, DenseWorld, VoxelModel};

/// The repository's `tests/` folder.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A file in `tests/golden/`.
pub fn golden(name: &str) -> PathBuf {
    root().join("golden").join(name)
}

/// A file in `tests/replays/`.
pub fn replay(name: &str) -> PathBuf {
    root().join("replays").join(name)
}

/// A file in `tests/scenes/`.
pub fn scene_path(name: &str) -> PathBuf {
    root().join("scenes").join(name)
}

/// Loads a small voxel scene.
///
/// Format, one directive per line, `#` starts a comment:
///
/// ```text
/// bounds <min x> <min y> <min z> <size x> <size y> <size z>
/// voxel <x> <y> <z> <material>
/// fill <x0> <y0> <z0> <x1> <y1> <z1> <material>   (inclusive corners)
/// ```
pub fn load_scene(name: &str) -> DenseWorld {
    let path = scene_path(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read scene {}: {e}", path.display()));
    parse_scene(&text).unwrap_or_else(|e| panic!("bad scene {}: {e}", path.display()))
}

pub fn parse_scene(text: &str) -> Result<DenseWorld, String> {
    let mut world: Option<DenseWorld> = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let directive = words.next().unwrap_or("");
        let numbers = words
            .map(|w| {
                w.parse::<i32>()
                    .map_err(|e| format!("line {}: {w:?}: {e}", n + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        match (directive, numbers.as_slice()) {
            ("bounds", &[x, y, z, sx, sy, sz]) => {
                world = Some(DenseWorld::new(Bounds::new(
                    VoxelPos::new(x, y, z),
                    [sx as u32, sy as u32, sz as u32],
                )));
            }
            ("voxel", &[x, y, z, m]) => {
                let w = world.as_mut().ok_or("`bounds` must come first")?;
                w.set(VoxelPos::new(x, y, z), MaterialId(m as u16));
            }
            ("fill", &[x0, y0, z0, x1, y1, z1, m]) => {
                let w = world.as_mut().ok_or("`bounds` must come first")?;
                for z in z0..=z1 {
                    for y in y0..=y1 {
                        for x in x0..=x1 {
                            w.set(VoxelPos::new(x, y, z), MaterialId(m as u16));
                        }
                    }
                }
            }
            _ => return Err(format!("line {}: cannot understand {raw:?}", n + 1)),
        }
    }
    world.ok_or_else(|| "scene has no `bounds` line".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bounds_voxels_and_fills() {
        let w =
            parse_scene("bounds 0 0 0 4 4 4\nvoxel 1 1 1 3  # one\nfill 2 2 2 3 3 3 5\n").unwrap();
        assert_eq!(w.get(VoxelPos::new(1, 1, 1)), MaterialId(3));
        assert_eq!(w.get(VoxelPos::new(3, 3, 3)), MaterialId(5));
        assert_eq!(w.solid_count(), 1 + 8);
    }

    #[test]
    fn voxel_before_bounds_is_an_error() {
        assert!(parse_scene("voxel 0 0 0 1").is_err());
    }

    #[test]
    fn garbage_is_rejected_with_its_line_number() {
        let err = parse_scene("bounds 0 0 0 1 1 1\nwhat 1 2").unwrap_err();
        assert!(err.contains("line 2"));
    }
}
