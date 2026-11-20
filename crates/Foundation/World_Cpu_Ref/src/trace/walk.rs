//! The traversal loop: enter the cube, then alternate "find the cell the
//! current voxel is in" (descending from the deepest ancestor still valid)
//! and "leave that empty cell", until a solid cell, the far side of the cube
//! or `max_t`.

use world_core::tree::{NodeKind, NodeView};
use world_core::{MaterialId, VoxelPos, VoxelSource};

use super::axis::{Axis, before};
use super::coarse::representative;
use super::{Face, MAX_LEVELS, TraceHit, TraceOptions};
use crate::camera::Ray;

/// What the cell around the current voxel is.
enum Cell {
    /// Solid at the stop level, with its material.
    Solid(MaterialId),
    /// The aligned cell of this level (`4^level` voxels wide) is all air.
    Empty { level: u32 },
}

/// The nodes on the path to the current voxel: `stack[i]` is the node of
/// tree level `levels - 1 - i` that contains it, valid for `i < depth`.
struct Path<N: NodeView> {
    stack: [Option<N>; MAX_LEVELS as usize],
    depth: usize,
    levels: u32,
    stop: u32,
}

impl<N: NodeView> Path<N> {
    fn new(root: N, levels: u32, stop: u32) -> Self {
        let mut stack = [None; MAX_LEVELS as usize];
        stack[0] = Some(root);
        Self {
            stack,
            depth: 1,
            levels,
            stop,
        }
    }

    /// The cell holding cube-local (unmirrored) voxel `local`, descending
    /// from the deepest valid ancestor.
    fn cell(&mut self, local: [i64; 3]) -> Cell {
        if self.stop == self.levels {
            // The whole cube is one cell; its "mask bit" is whether it is air.
            let root = self.node(0);
            return match root.kind() {
                NodeKind::Air => Cell::Empty { level: self.levels },
                NodeKind::Uniform(m) => Cell::Solid(m),
                NodeKind::Branch | NodeKind::Leaf => {
                    Cell::Solid(representative(root, self.levels - 1))
                }
            };
        }
        loop {
            let node = self.node(self.depth - 1);
            let level = self.levels - self.depth as u32;
            // TODO(NotResident, decision 0006): when `NodeKind::NotResident`
            // lands, add one arm here returning a not-resident cell of
            // `level + 1` (cell and level, no material) and report it as a
            // distinct hit; never treat it as air. The match is kept
            // exhaustive so the new variant cannot slip through unnoticed.
            match node.kind() {
                NodeKind::Air => return Cell::Empty { level: level + 1 },
                NodeKind::Uniform(m) => return Cell::Solid(m),
                NodeKind::Branch | NodeKind::Leaf => {
                    let slot = slot(local, level);
                    if node.mask() & (1u64 << slot) == 0 {
                        return Cell::Empty { level };
                    }
                    if level == self.stop {
                        // The child is the stop cell. At level 0 it is a voxel.
                        return Cell::Solid(if level == 0 {
                            node.voxel(slot)
                        } else {
                            representative(node.child(slot), level - 1)
                        });
                    }
                    self.stack[self.depth] = Some(node.child(slot));
                    self.depth += 1;
                }
            }
        }
    }

    /// Stack entry `i`, which is always set below `depth`.
    fn node(&self, i: usize) -> N {
        match self.stack[i] {
            Some(node) => node,
            // Invariant: entries 0..depth are written before depth grows past
            // them, and depth never drops below 1.
            None => unreachable!("path entry {i} below depth {} is unset", self.depth),
        }
    }

    /// Keep only the ancestors that contain both `old` and `new` (mirrored or
    /// not: mirroring is an XOR with `size - 1` inside the cube, so the XOR
    /// of two positions is the same). A node of level `L` covers `4^(L+1)`
    /// voxels, so it contains both when `old ^ new` has no bit at `2(L+1)`
    /// or above.
    fn moved(&mut self, old: [i64; 3], new: [i64; 3]) {
        let diff = ((old[0] ^ new[0]) | (old[1] ^ new[1]) | (old[2] ^ new[2])) as u64;
        if diff == 0 {
            return;
        }
        let highest = 63 - diff.leading_zeros();
        let lowest_common_level = highest / 2;
        let keep = (self.levels - lowest_common_level) as usize;
        self.depth = self.depth.min(keep.max(1));
    }
}

/// The child slot of cube-local `local` in a node of `level`:
/// `x + 4y + 16z` of the two bits at `2 * level` (decision 0004).
fn slot(local: [i64; 3], level: u32) -> u32 {
    let bits = |v: i64| ((v >> (2 * level)) & 3) as u32;
    bits(local[0]) | bits(local[1]) << 2 | bits(local[2]) << 4
}

/// The trace itself. Inputs are already validated by `trace`.
pub(super) fn walk<S: VoxelSource + ?Sized>(
    source: &S,
    ray: &Ray,
    options: &TraceOptions,
) -> Option<TraceHit> {
    let levels = source.levels();
    let size = 1i64 << (2 * levels);
    let min = source.origin().to_array();
    let axes: [Axis; 3] =
        std::array::from_fn(|a| Axis::new(a, ray.origin[a], ray.dir[a], i64::from(min[a]), size));
    let inside = |v: &[i64; 3]| v.iter().all(|c| (0..size).contains(c));

    let mut v: [i64; 3] = std::array::from_fn(|a| axes[a].start_voxel());
    // The crossing that brought the walk into `v`, as `(t, axis)`.
    let mut last: Option<(f32, usize)> = None;
    if !inside(&v) {
        let (entered, t, axis) = enter(&axes, v, size)?;
        if t > options.max_t {
            return None;
        }
        v = entered;
        last = Some((t, axis));
    }

    let mut path = Path::new(source.root(), levels, options.stop_level);
    loop {
        let local: [i64; 3] = std::array::from_fn(|a| axes[a].mirror(v[a]));
        let level = match path.cell(local) {
            Cell::Solid(material) => return Some(hit(&axes, v, last, material, options)),
            Cell::Empty { level } => level,
        };

        // Leave the empty cell: the first of its far planes, ordered by
        // (t, axis). Only moving axes have far planes.
        let mut exit: Option<(f32, usize, i64)> = None;
        for (a, axis) in axes.iter().enumerate() {
            if !axis.moving {
                continue;
            }
            let plane = ((v[a] >> (2 * level)) + 1) << (2 * level);
            let t = axis.crossing(plane);
            if exit.is_none_or(|(et, ea, _)| before(t, a, et, ea)) {
                exit = Some((t, a, plane));
            }
        }
        // A trace's direction is never zero, so some axis moves.
        let (t, a, plane) = exit?;
        if t > options.max_t || plane >= size {
            return None;
        }
        let old = v;
        for (b, axis) in axes.iter().enumerate() {
            if b != a && axis.moving {
                let cell_last = (((v[b] >> (2 * level)) + 1) << (2 * level)) - 1;
                v[b] = axis.voxel_at(v[b], cell_last, t, a);
            }
        }
        v[a] = plane;
        last = Some((t, a));
        path.moved(old, v);
    }
}

/// From a start voxel outside the cube, the voxel where the ray enters it and
/// the entering crossing `(t, axis)`: the last of the near-plane crossings in
/// `(t, axis)` order. `None` when the ray never enters: an axis outside the
/// cube that does not move toward it, or another axis leaving the cube before
/// the entry.
fn enter(axes: &[Axis; 3], v: [i64; 3], size: i64) -> Option<([i64; 3], f32, usize)> {
    let mut entry: Option<(f32, usize)> = None;
    for (a, axis) in axes.iter().enumerate() {
        if (0..size).contains(&v[a]) {
            continue;
        }
        // Mirrored axes all move `+`, so only `v < 0` can still come in.
        if !axis.moving || v[a] >= size {
            return None;
        }
        let t = axis.crossing(0);
        if entry.is_none_or(|(et, ea)| before(et, ea, t, a)) {
            entry = Some((t, a));
        }
    }
    let (t, a) = entry?;
    let mut entered = v;
    entered[a] = 0;
    for (b, axis) in axes.iter().enumerate() {
        if b != a && axis.moving {
            // `size` itself means "already left through the far side".
            entered[b] = axis.voxel_at(v[b], size, t, a);
            if entered[b] >= size {
                return None;
            }
        }
    }
    Some((entered, t, a))
}

/// The hit record for solid cell around mirrored voxel `v`.
fn hit(
    axes: &[Axis; 3],
    v: [i64; 3],
    last: Option<(f32, usize)>,
    material: MaterialId,
    options: &TraceOptions,
) -> TraceHit {
    // Inside the cube, so every coordinate fits in i32 (the source's own
    // bounds are i32).
    let world: [i32; 3] = std::array::from_fn(|a| axes[a].world(v[a]) as i32);
    let (face, t) = match last {
        Some((t, a)) => (Face::entered(a, axes[a].positive()), t),
        None => (Face::Inside, 0.0),
    };
    TraceHit {
        voxel: VoxelPos::new(world[0], world[1], world[2]),
        level: options.stop_level,
        face,
        material,
        t,
    }
}
