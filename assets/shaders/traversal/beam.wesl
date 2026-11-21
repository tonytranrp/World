// Coarse beam traversal (decision 0023): the nearest distance at which any
// ray of a bundle could meet something, found once for the whole bundle so
// each ray can start there instead of at its origin.
//
// The bundle is every ray `o + t d` whose unit direction `d` lies within
// `spread` (a chord length, |d - c|) of the unit axis `c`. At distance `t`
// all its points lie in the cube of half-width `r(t) = k1 t + k0` around
// `o + c t`, where `k1` is the spread plus a rounding allowance and `k0` a
// constant margin of about half a voxel. Both are linear in `t`, so each
// face of the cube moves linearly too.
//
// A step at distance `t`:
//
// - Pick the level `L` whose cells are at least as large as the cube (with
//   a small growth) and as large as the pixel footprint at `t`. The cube
//   then overlaps at most two level-`L` cells on each axis.
// - Look up each of those cells (up to eight) from the root. A cell lies in
//   an empty cell of some level `e >= L` (a clear mask bit), or it is
//   occupied: a set bit at level `L`, a uniform node, or a brick or
//   not-resident node whose bit is set (their children are never read).
//   One occupied cell ends the march at `t`.
// - The empty cells hold a box `V` around the cube: on an axis the cube
//   does not straddle, the overlap of every cell's extent; on one it
//   straddles, the low cells' lower bound and the high cells' upper bound.
//   Each corner region of `V` lies in its own cell's empty cell, so `V` is
//   empty at every level from `L` down.
// - Advance to the first distance at which a face of the cube reaches a
//   face of `V`, and no further than where the footprint would let a
//   per-pixel ray stop at an ancestor of the smallest empty cell found.
//
// What a ray started at the result sees (`cast_ray_from` in `tree.wesl`):
// every cell it would have passed before that distance is empty at a level
// no finer than the one the per-pixel trace stops at, so it would have
// stopped nowhere, requested nothing and stamped no brick there, and the
// coarse-detail test at the restart cannot fire where it did not before.
// Its hit is therefore the one it gets from its origin. Only 32-bit
// arithmetic, no infinities.

import package::shaders::common::format::{Node, node_kind, mask_bit, mask_rank, child_slot};
import package::shaders::common::constants::{KIND_INTERNAL, KIND_UNIFORM, ROOT_NODE};
import package::shaders::traversal::pool::load_node;

// The lookup result of an occupied cell.
const BEAM_OCCUPIED: u32 = 0xFFFFFFFFu;
// Growth of the cube, in voxels, when choosing cells: a cube that just
// touches a cell boundary also takes the cell beyond it, so a step always
// makes progress.
const BEAM_GROW: f32 = 0.25;

struct BeamSettings {
    // The tree's lowest voxel and its root level (levels - 1).
    origin: vec3<i32>,
    root_level: u32,
    // Pixel footprint per unit distance (radians per pixel times the
    // footprint bias): where the per-pixel trace stops at coarse cells.
    // 0 traces at full resolution.
    footprint: f32,
    max_steps: u32,
}

// The level of the empty cell holding the level-`min_level` cell at local
// voxel `p` (tree frame), looked up from node `index` of level `level`,
// or `BEAM_OCCUPIED`.
fn beam_lookup(index: u32, level: u32, p: vec3<u32>, min_level: u32) -> u32 {
    var node = load_node(index);
    var n = level;
    loop {
        let kind = node_kind(node);
        if kind == KIND_UNIFORM {
            return BEAM_OCCUPIED;
        }
        let slot = child_slot(p, n);
        if !mask_bit(node.mask_lo, node.mask_hi, slot) {
            return n;
        }
        if n <= min_level || kind != KIND_INTERNAL {
            return BEAM_OCCUPIED;
        }
        node = load_node(node.reference + mask_rank(node.mask_lo, node.mask_hi, slot));
        n -= 1u;
    }
    return BEAM_OCCUPIED;
}

// The distance from which every ray of the bundle may start (see the top of
// this file). `o` and the result are in voxels, world frame; `c` is unit.
fn beam_start(o: vec3<f32>, c: vec3<f32>, spread: f32, s: BeamSettings) -> f32 {
    if s.root_level >= 10u || s.max_steps == 0u {
        return 0.0;
    }
    let size = 1u << (2u * (s.root_level + 1u));
    let lo = vec3<f32>(s.origin);
    // Directions and distances are rounded to a few ulps; a hundred times
    // that per unit distance, plus half a voxel and as much again for the
    // coordinates' own rounding, keeps every ray's computed voxels inside.
    let k1 = spread * 1.0001 + 1.0e-5;
    let k0 = 0.5 + 1.0e-5 * max(abs(o.x), max(abs(o.y), abs(o.z)));
    var t = 0.0;
    for (var i = 0u; i < s.max_steps; i++) {
        let r = k1 * t + k0;
        let q = o + c * t - lo;
        let bmin = q - vec3<f32>(r + BEAM_GROW);
        let bmax = q + vec3<f32>(r + BEAM_GROW);
        if any(bmin < vec3<f32>(0.0)) || any(bmax >= vec3<f32>(f32(size))) {
            return t;
        }
        // Cells at least as wide as the grown cube and the footprint.
        let need = max(2.0 * (r + BEAM_GROW), t * s.footprint * 1.001);
        var level = 1u;
        while level < s.root_level && f32(1u << (2u * level)) < need {
            level += 1u;
        }
        if f32(1u << (2u * level)) < need {
            return t;
        }
        let shift = vec3<u32>(2u * level);
        let cmin = (vec3<u32>(floor(bmin)) >> shift) << shift;
        let cmax = (vec3<u32>(floor(bmax)) >> shift) << shift;
        let straddle = cmax != cmin;

        // Descend once along the path the cells share: node `index` of
        // level `n` holds them all.
        var index = ROOT_NODE;
        var n = s.root_level;
        loop {
            let together = (cmin >> vec3<u32>(2u * n)) == (cmax >> vec3<u32>(2u * n));
            if n <= level || !all(together) {
                break;
            }
            let node = load_node(index);
            let kind = node_kind(node);
            let slot = child_slot(cmin, n);
            if kind == KIND_UNIFORM {
                return t;
            }
            if !mask_bit(node.mask_lo, node.mask_hi, slot) {
                break;
            }
            if kind != KIND_INTERNAL {
                return t;
            }
            index = node.reference + mask_rank(node.mask_lo, node.mask_hi, slot);
            n -= 1u;
        }

        // Each cell's empty cell, folded into the box V (tree frame).
        var vlo = vec3<u32>(0u);
        var vhi = vec3<u32>(size);
        var e_min = s.root_level;
        let span = 1u << (2u * level);
        for (var k = 0u; k < 8u; k++) {
            let side = vec3<u32>(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
            if any((side == vec3<u32>(1u)) & !straddle) {
                continue;
            }
            let cell = cmin + side * span;
            let e = beam_lookup(index, n, cell, level);
            if e == BEAM_OCCUPIED {
                return t;
            }
            e_min = min(e_min, e);
            let es = 2u * e;
            let elo = (cell >> vec3<u32>(es)) << vec3<u32>(es);
            let ehi = elo + vec3<u32>(1u << es);
            let low = !straddle | (side == vec3<u32>(0u));
            let high = !straddle | (side == vec3<u32>(1u));
            vlo = select(vlo, max(vlo, elo), low);
            vhi = select(vhi, min(vhi, ehi), high);
        }

        // The first distance at which a face of the cube reaches V.
        let wlo = vec3<f32>(vlo) + lo;
        let whi = vec3<f32>(vhi) + lo;
        let up = c + vec3<f32>(k1);
        let down = vec3<f32>(k1) - c;
        let t_up = select(vec3<f32>(3.0e38), (whi - o - vec3<f32>(k0)) / up, up > vec3<f32>(0.0));
        let t_down = select(vec3<f32>(3.0e38), (o - vec3<f32>(k0) - wlo) / down, down > vec3<f32>(0.0));
        var next = min(min(t_up.x, t_up.y), min(min(t_up.z, t_down.x), min(t_down.y, t_down.z)));
        if s.footprint > 0.0 {
            // A per-pixel ray restarting here must not stop at an ancestor
            // of the smallest empty cell: its cells are at least 4^(e+1).
            next = min(next, f32(1u << (2u * e_min + 2u)) / s.footprint * 0.999);
        }
        if !(next > t) {
            return t;
        }
        t = next;
    }
    return t;
}
