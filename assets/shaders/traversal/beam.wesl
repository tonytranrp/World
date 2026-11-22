// Coarse beam traversal (decisions 0023 and 0026): the nearest distance at
// which any ray of a bundle could meet something, found once for the whole
// bundle so each ray can start there instead of at its origin, or the proof
// that none of them meets anything before leaving the tree.
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
// - If the whole cube lies past a face of the tree and moves away from it,
//   every ray has left the tree: the bundle is sky (`BEAM_SKY`). This needs
//   the origin inside the tree, so that a ray found outside has left it for
//   good; with the origin outside the march does not run.
// - Clip the cube (with a small growth) to the tree; outside is empty. Pick
//   the level `L` whose cells are at least as large as the clipped cube and
//   as the pixel footprint at `t`. The cube then overlaps at most two
//   level-`L` cells on each axis.
// - Look up each of those cells (up to eight) from the root. A cell lies in
//   an empty cell of some level `e >= L` (a clear mask bit), or it is
//   occupied: a set bit at level `L`, a uniform node, or a brick or
//   not-resident node whose bit is set (their children are never read).
// - An occupied cell that is a set bit of an internal node, with `L >= 2`,
//   is looked into once: if the children (level `L - 1`) the cube overlaps
//   are all clear, their box, grown along the bundle's motion while it stays
//   clear, counts as the cell's empty cell, of level `e = L - 1`. Any other
//   occupied cell ends the march at `t`.
// - The empty cells hold a box `V` around the cube: on an axis the cube
//   does not straddle, the overlap of every cell's extent; on one it
//   straddles, the low cells' lower bound and the high cells' upper bound.
//   Each corner region of `V` lies in its own cell's empty cell, so `V` is
//   empty at every level from `L - 1` down. Past a face of the tree the cube
//   crosses, `V` reaches far out.
// - Advance to the first distance at which a face of the cube reaches a
//   face of `V`, and no further than where the footprint would let a
//   per-pixel ray stop at an ancestor of the smallest empty cell found.
//
// What a ray started at the result sees (`cast_ray_from` in `tree.wesl`):
// every cell it would have passed before that distance is empty at a level
// no finer than the one the per-pixel trace stops at, so it would have
// stopped nowhere, requested nothing and stamped no brick there, and the
// coarse-detail test at the restart cannot fire where it did not before.
// Its hit is therefore the one it gets from its origin; past the tree's
// face it is a miss. Only 32-bit arithmetic, no infinities.

import package::shaders::common::format::{Node, node_kind, mask_bit, mask_rank, child_slot};
import package::shaders::common::constants::{KIND_INTERNAL, KIND_UNIFORM, ROOT_NODE};
import package::shaders::traversal::pool::load_node;

// The lookup result of an occupied cell.
const BEAM_OCCUPIED: u32 = 0xFFFFFFFFu;
// Growth of the cube, in voxels, when choosing cells: a cube that just
// touches a cell boundary also takes the cell beyond it, so a step always
// makes progress.
const BEAM_GROW: f32 = 0.25;
// What `beam_start` returns when every ray of the bundle leaves the tree
// without stopping: the trace writes a miss for them at once.
const BEAM_SKY: f32 = -1.0;
// A coordinate far outside any tree, where the free box reaches past the
// tree's faces (finite: no infinities).
const BEAM_BEYOND: f32 = 1.0e20;

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

// What `beam_lookup` finds for one cell.
struct BeamCell {
    // The level of the empty cell holding it, or `BEAM_OCCUPIED`.
    level: u32,
    // When occupied by a set bit of an internal node of the cell's own
    // level: the pool index of that bit's child node, whose cells are a
    // level finer. Otherwise `BEAM_OCCUPIED`.
    child: u32,
}

// The empty cell holding the level-`min_level` cell at local voxel `p`
// (tree frame), looked up from node `index` of level `level`.
fn beam_lookup(index: u32, level: u32, p: vec3<u32>, min_level: u32) -> BeamCell {
    var node = load_node(index);
    var n = level;
    loop {
        let kind = node_kind(node);
        if kind == KIND_UNIFORM {
            break;
        }
        let slot = child_slot(p, n);
        if !mask_bit(node.mask_lo, node.mask_hi, slot) {
            return BeamCell(n, BEAM_OCCUPIED);
        }
        let child = node.reference + mask_rank(node.mask_lo, node.mask_hi, slot);
        if kind != KIND_INTERNAL {
            break;
        }
        if n <= min_level {
            return BeamCell(BEAM_OCCUPIED, child);
        }
        node = load_node(child);
        n -= 1u;
    }
    return BeamCell(BEAM_OCCUPIED, BEAM_OCCUPIED);
}

// Whether no bit of a node's mask (slot x | y << 2 | z << 4) lies in the box
// of cells `a` to `b` (inclusive, each 0 to 3).
fn beam_box_clear(mask_lo: u32, mask_hi: u32, a: vec3<u32>, b: vec3<u32>) -> bool {
    let rx = ((2u << b.x) - (1u << a.x)) * 0x11111111u;
    let ry = ((1u << (4u * b.y + 4u)) - (1u << (4u * a.y))) * 0x00010001u;
    let z_lo = select(0u, 0xFFFFu, a.z == 0u) | select(0u, 0xFFFF0000u, a.z <= 1u && b.z >= 1u);
    let z_hi = select(0u, 0xFFFFu, a.z <= 2u && b.z >= 2u) | select(0u, 0xFFFF0000u, b.z == 3u);
    let xy = rx & ry;
    return ((mask_lo & xy & z_lo) | (mask_hi & xy & z_hi)) == 0u;
}

// The distance from which every ray of the bundle may start (see the top of
// this file), or `BEAM_SKY` when none of them stops anywhere. `o` and the
// result are in voxels, world frame; `c` is unit.
fn beam_start(o: vec3<f32>, c: vec3<f32>, spread: f32, s: BeamSettings) -> f32 {
    if s.root_level >= 10u || s.max_steps == 0u {
        return 0.0;
    }
    let size = 1u << (2u * (s.root_level + 1u));
    let size_f = f32(size);
    let lo = vec3<f32>(s.origin);
    // Directions and distances are rounded to a few ulps; a hundred times
    // that per unit distance, plus half a voxel and as much again for the
    // coordinates' own rounding, keeps every ray's computed voxels inside.
    let k1 = spread * 1.0001 + 1.0e-5;
    let k0 = 0.5 + 1.0e-5 * max(abs(o.x), max(abs(o.y), abs(o.z)));
    // Outside the tree counts as empty only for rays that start inside it:
    // a ray found outside has then left for good, and the trace ends it.
    let eye = o - lo;
    if any(eye < vec3<f32>(k0 + 1.0)) || any(eye > vec3<f32>(size_f - k0 - 1.0)) {
        return 0.0;
    }
    var t = 0.0;
    for (var i = 0u; i < s.max_steps; i++) {
        let r = k1 * t + k0;
        let q = o + c * t - lo;
        // The whole cube is past a face of the tree, a voxel clear of it,
        // and that face of the cube moves outward: every ray of the bundle
        // has left the tree, and every cell it passed before was empty.
        let gone_hi = (q - vec3<f32>(r) > vec3<f32>(size_f + 1.0)) & (c >= vec3<f32>(k1));
        let gone_lo = (q + vec3<f32>(r) < vec3<f32>(-1.0)) & (c <= vec3<f32>(-k1));
        if any(gone_hi | gone_lo) {
            return BEAM_SKY;
        }
        let bmin = q - vec3<f32>(r + BEAM_GROW);
        let bmax = q + vec3<f32>(r + BEAM_GROW);
        // The part of the grown cube inside the tree; what lies outside it
        // is empty.
        let out_lo = bmin < vec3<f32>(0.0);
        let out_hi = bmax >= vec3<f32>(size_f);
        let edge = vec3<f32>(size_f - 1.0);
        let in_min = clamp(bmin, vec3<f32>(0.0), edge);
        let in_max = clamp(bmax, vec3<f32>(0.0), edge);
        let width = in_max - in_min;
        // Cells at least as wide as that part and the footprint.
        let need = max(max(width.x, max(width.y, width.z)), t * s.footprint * 1.001);
        var level = 1u;
        while level < s.root_level && f32(1u << (2u * level)) < need {
            level += 1u;
        }
        if f32(1u << (2u * level)) < need {
            return t;
        }
        let shift = vec3<u32>(2u * level);
        let cmin = (vec3<u32>(floor(in_min)) >> shift) << shift;
        let cmax = (vec3<u32>(floor(in_max)) >> shift) << shift;
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
            let found = beam_lookup(index, n, cell, level);
            var e = found.level;
            var elo: vec3<u32>;
            var ehi: vec3<u32>;
            if e == BEAM_OCCUPIED {
                // A set bit of an internal node of level L >= 2: the per-pixel
                // trace descends into it (the cap below keeps its footprint
                // test from firing), so look at its children, a level finer.
                // The ones the grown cube overlaps must all be clear; the box
                // they make then grows along the bundle's motion while it
                // stays clear, and stands in for the cell's empty cell.
                if found.child == BEAM_OCCUPIED || level < 2u {
                    return t;
                }
                let child = load_node(found.child);
                if node_kind(child) == KIND_UNIFORM {
                    return t;
                }
                let cs = vec3<u32>(2u * (level - 1u));
                let last = cell + vec3<u32>(span - 1u);
                var a = (clamp(vec3<u32>(floor(in_min)), cell, last) - cell) >> cs;
                var b = (clamp(vec3<u32>(floor(in_max)), cell, last) - cell) >> cs;
                let m_lo = child.mask_lo;
                let m_hi = child.mask_hi;
                if !beam_box_clear(m_lo, m_hi, a, b) {
                    return t;
                }
                let ux = vec3<u32>(1u, 0u, 0u);
                let uy = vec3<u32>(0u, 1u, 0u);
                let uz = vec3<u32>(0u, 0u, 1u);
                for (var g = 0u; g < 3u; g++) {
                    if c.x > 0.0 && b.x < 3u && beam_box_clear(m_lo, m_hi, a, b + ux) {
                        b += ux;
                    }
                    if c.x < 0.0 && a.x > 0u && beam_box_clear(m_lo, m_hi, a - ux, b) {
                        a -= ux;
                    }
                    if c.y > 0.0 && b.y < 3u && beam_box_clear(m_lo, m_hi, a, b + uy) {
                        b += uy;
                    }
                    if c.y < 0.0 && a.y > 0u && beam_box_clear(m_lo, m_hi, a - uy, b) {
                        a -= uy;
                    }
                    if c.z > 0.0 && b.z < 3u && beam_box_clear(m_lo, m_hi, a, b + uz) {
                        b += uz;
                    }
                    if c.z < 0.0 && a.z > 0u && beam_box_clear(m_lo, m_hi, a - uz, b) {
                        a -= uz;
                    }
                }
                e = level - 1u;
                elo = cell + (a << cs);
                ehi = cell + ((b + vec3<u32>(1u)) << cs);
            } else {
                let es = vec3<u32>(2u * e);
                elo = (cell >> es) << es;
                ehi = elo + (vec3<u32>(1u) << es);
            }
            e_min = min(e_min, e);
            let low = !straddle | (side == vec3<u32>(0u));
            let high = !straddle | (side == vec3<u32>(1u));
            vlo = select(vlo, max(vlo, elo), low);
            vhi = select(vhi, min(vhi, ehi), high);
        }

        // The first distance at which a face of the cube reaches V.
        // Beyond the tree's faces the cube crosses, V reaches far out.
        let wlo = select(vec3<f32>(vlo) + lo, vec3<f32>(-BEAM_BEYOND), out_lo);
        let whi = select(vec3<f32>(vhi) + lo, vec3<f32>(BEAM_BEYOND), out_hi);
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
