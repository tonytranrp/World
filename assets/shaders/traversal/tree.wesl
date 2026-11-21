// 64-tree traversal over the GPU pools (decision 0007), shared by every
// pass that casts a ray.
//
// The ray walks the same voxel sequence as the CPU reference DDA
// (World_Testkit `DenseWorld::cast_ray`), but skips empty cells of any
// level at once:
//
// - Octant mirroring: axes with a negative direction are mirrored
//   (x' = -x, voxel v' = -v - 1), so every step is +1. Negation is exact
//   in IEEE arithmetic, so `(boundary - origin) / dir` gives bit-identical
//   distances in both frames.
// - Masks: a clear bit in a node's 64-bit mask is an empty cell of that
//   level; the ray jumps to the cell's exit. The axis it leaves through is
//   the one with the smallest boundary distance, the lowest axis on ties,
//   exactly the reference's rule, and the voxel it lands in is found by
//   counting the boundaries each other axis crossed by then, with the same
//   tie rule. Distances are computed directly from integer boundaries,
//   never accumulated.
// - Whole-node skips: in an empty cell, if the box of cells between it and
//   the node's exit holds no set bit, the ray jumps to the node's exit at
//   once instead of crossing up to ten empty cells one by one (decision
//   0015: steps per ray fell from about 55 to 21 on the floor walk).
// - Ancestor stack: after a jump, descent restarts at the deepest node that
//   still contains the new voxel instead of at the root.
//
// Descent stops where a cell is smaller than the pixel footprint (scaled by
// the bias; a bias of 0 is full resolution) or where a node's children are
// not resident; the hit then reports that cell's level and the node's
// representative material, and a not-resident node is requested once per
// frame when finer detail is wanted. Only 32-bit atomics are used, and no
// infinities: `T_FAR` stands in where the CPU reference uses infinity.

import package::shaders::common::format::{
    Node, node_kind, node_material, mask_bit, mask_rank, child_slot,
};
import package::shaders::common::constants::{
    KIND_UNIFORM, KIND_NOT_RESIDENT, KIND_BRICK, ROOT_NODE,
    BLOCK_HEADER_WORDS, BLOCK_COUNT_MASK, LEAF_MASK_WORDS, PAYLOAD_UNIFORM_BIT,
    PALETTE_LEN_MASK, PALETTE_BITS_SHIFT, BRICK_TABLE_WORDS,
    FEEDBACK_HEADER_WORDS, REQUEST_WORDS, REQUEST_LEVEL_SHIFT,
    REQUEST_PRIORITY_SHIFT, REQUEST_PRIORITY_MAX,
    HIT_FLAG_HIT, HIT_FLAG_RESIDENCY, HIT_FLAG_FOOTPRINT, HIT_FLAG_EXHAUSTED, FACE_INSIDE,
};
import package::shaders::traversal::pool::{
    load_node, block_segment, block_base, brick_word, pool_table, pool_feedback,
};

// A large finite distance: WGSL has no infinities (decision 0005).
const T_FAR: f32 = 3.0e38;

struct TraceSettings {
    // The tree's lowest voxel and its root level (levels - 1).
    origin: vec3<i32>,
    root_level: u32,
    // This frame's number, 1 or more: stamps and requests compare it.
    frame: u32,
    // Radians per pixel, and the footprint bias (0: full resolution).
    pixel_angle: f32,
    footprint_bias: f32,
    request_capacity: u32,
    // Word offset of the per-node request stamps in the feedback buffer.
    stamps_offset: u32,
    max_iterations: u32,
}

struct Hit {
    // The first voxel of the hit cell the ray entered, world coordinates.
    voxel: vec3<i32>,
    material: u32,
    // Cell size the hit was decided at: 4^level voxels.
    level: u32,
    // 0 -x, 1 +x, 2 -y, 3 +y, 4 -z, 5 +z, 6 inside.
    face: u32,
    flags: u32,
    t: f32,
    steps: u32,
}

fn trace_miss(steps: u32, flags: u32) -> Hit {
    return Hit(vec3<i32>(0), 0u, 0u, 0u, flags, T_FAR, steps);
}

fn trace_hit(
    v: vec3<i32>, m: vec3<bool>, material: u32, level: u32, face_axis: u32,
    flags: u32, t: f32, steps: u32,
) -> Hit {
    var face = FACE_INSIDE;
    if face_axis < 3u {
        // A +1 step in the mirrored frame is a -1 step on a mirrored axis,
        // which enters through the + face.
        let mirrored = select(select(m.z, m.y, face_axis == 1u), m.x, face_axis == 0u);
        face = 2u * face_axis + select(0u, 1u, mirrored);
    }
    let voxel = select(v, -v - vec3<i32>(1), m);
    return Hit(voxel, material, level, face, flags, t, steps);
}

// The voxel index on one axis once every boundary with distance below `t`
// (at most `t` when `inclusive`) has been crossed, starting from voxel
// `start`; the reference's state at the moment of a step at distance `t`.
fn axis_voxel(start: i32, od: f32, dd: f32, t: f32, inclusive: bool) -> i32 {
    if dd <= 0.0 {
        return start;
    }
    var g = max(i32(floor(od + dd * t)), start);
    for (var i = 0; i < 8; i++) {
        let tb = (f32(g + 1) - od) / dd;
        if select(tb < t, tb <= t, inclusive) {
            g += 1;
        } else {
            break;
        }
    }
    for (var i = 0; i < 8; i++) {
        if g <= start {
            break;
        }
        let tb = (f32(g) - od) / dd;
        if select(tb < t, tb <= t, inclusive) {
            break;
        }
        g -= 1;
    }
    return g;
}

// Marks a brick used this frame: a plain store, guarded by a read so each
// table entry is written at most once per frame.
fn stamp_brick(segment: u32, base: u32, frame: u32) {
    let id = brick_word(segment, base);
    let at = id * BRICK_TABLE_WORDS + 1u;
    if at < arrayLength(&pool_table) && pool_table[at] != frame {
        pool_table[at] = frame;
    }
}

// Asks the CPU for the children (or brick) of the node at pool `index`, of
// `level`, containing `local`. At most once per node per frame.
fn request_node(index: u32, local: vec3<u32>, level: u32, t: f32, s: TraceSettings) {
    let stamp = s.stamps_offset + index;
    if stamp >= arrayLength(&pool_feedback) {
        return;
    }
    if atomicMax(&pool_feedback[stamp], s.frame) >= s.frame {
        return;
    }
    let slot = atomicAdd(&pool_feedback[0], 1u);
    if slot >= s.request_capacity {
        atomicAdd(&pool_feedback[1], 1u);
        return;
    }
    let shift = vec3<u32>(2u * (level + 1u));
    let corner = ((local >> shift) << shift) >> vec3<u32>(4u);
    let near = u32(sqrt(max(t, 0.0)) * 8.0);
    let priority = REQUEST_PRIORITY_MAX - min(REQUEST_PRIORITY_MAX, near);
    let at = FEEDBACK_HEADER_WORDS + slot * REQUEST_WORDS;
    atomicStore(&pool_feedback[at], corner.x | (corner.y << 16u));
    atomicStore(
        &pool_feedback[at + 1u],
        corner.z | (level << REQUEST_LEVEL_SHIFT) | (priority << REQUEST_PRIORITY_SHIFT),
    );
}

// The material of solid voxel `vslot` of leaf `k` of a block with `n` leaves.
fn leaf_material(
    segment: u32, base: u32, n: u32, k: u32, leaf_lo: u32, leaf_hi: u32, vslot: u32,
) -> u32 {
    let payload = brick_word(segment, base + BLOCK_HEADER_WORDS + n * LEAF_MASK_WORDS + k);
    if (payload & PAYLOAD_UNIFORM_BIT) != 0u {
        return payload & 0xFFFFu;
    }
    let p = base + payload;
    let header = brick_word(segment, p);
    let len = header & PALETTE_LEN_MASK;
    let bits = (header >> PALETTE_BITS_SHIFT) & 0xFFu;
    let bit = mask_rank(leaf_lo, leaf_hi, vslot) * bits;
    let indices = p + 1u + (len + 1u) / 2u;
    let word = brick_word(segment, indices + bit / 32u);
    let index = (word >> (bit % 32u)) & ((1u << bits) - 1u);
    let entry = brick_word(segment, p + 1u + index / 2u);
    return (entry >> (16u * (index & 1u))) & 0xFFFFu;
}

// Component `k` (0 to 2) of a vector. Dynamic indexing is avoided on purpose:
// D3D's FXC cannot address vector components or private arrays by a
// runtime index and gives up unrolling the loops around them.
fn pick_f(v: vec3<f32>, k: u32) -> f32 {
    return select(select(v.z, v.y, k == 1u), v.x, k == 0u);
}

fn pick_i(v: vec3<i32>, k: u32) -> i32 {
    return select(select(v.z, v.y, k == 1u), v.x, k == 0u);
}

// The voxel after a step on axis `k` into voxel `bk` (on that axis) at
// distance `t`: the other axes have crossed every boundary below `t`, and
// those of lower axes also the ones at exactly `t` (the reference's order).
fn step_voxel(v: vec3<i32>, k: u32, bk: i32, od: vec3<f32>, dd: vec3<f32>, t: f32) -> vec3<i32> {
    let x = select(axis_voxel(v.x, od.x, dd.x, t, k > 0u), bk, k == 0u);
    let y = select(axis_voxel(v.y, od.y, dd.y, t, k > 1u), bk, k == 1u);
    let z = select(axis_voxel(v.z, od.z, dd.z, t, false), bk, k == 2u);
    return vec3<i32>(x, y, z);
}

// Bits `a` to `b` (0 to 3, a <= b) of a 4-bit group.
fn bit_range4(a: u32, b: u32) -> u32 {
    return (2u << b) - (1u << a);
}

// Whether the ray leaves the node holding cell `ml >> 2 level` (mirrored
// local voxel coordinates) without entering a cell its mask marks solid.
// From here to the node's exit the mirrored coordinates only grow, so the
// cells it passes lie in the box from its current cell to the cell of its
// exit point; if no set bit falls in that box, every cell it would step
// through is empty and it may jump straight to the node's exit, which the
// caller computes exactly as for any empty cell. The box's upper corner is
// widened against rounding (half a voxel plus about eight ulps of the
// coordinate), so it is never smaller than the true one: skipping is exact,
// only sometimes not taken. Skipped cells have no side effects (requests and
// brick stamps come only from cells with set bits).
fn rest_of_node_empty(
    mask_lo: u32, mask_hi: u32, ml: vec3<u32>, level: u32, m: vec3<bool>,
    mo: vec3<i32>, od: vec3<f32>, dd: vec3<f32>,
) -> bool {
    let shift = 2u * level;
    let node_span = 1u << (shift + 2u);
    let node_end = vec3<f32>(vec3<i32>((ml | vec3<u32>(node_span - 1u)) + vec3<u32>(1u)) + mo);
    let tn = select(vec3<f32>(T_FAR), (node_end - od) / dd, dd > vec3<f32>(0.0));
    let te = min(tn.x, min(tn.y, tn.z));
    // Exit point relative to the node's lowest corner, in cells.
    let corner = vec3<f32>(vec3<i32>(ml & ~vec3<u32>(node_span - 1u)) + mo);
    let exit = od + dd * te;
    let slack = vec3<f32>(0.5) + abs(exit) * 1.0e-6;
    let rel = (exit - corner + slack) / f32(1u << shift);
    let c0 = (ml >> vec3<u32>(shift)) & vec3<u32>(3u);
    let c1 = max(min(vec3<u32>(max(floor(rel), vec3<f32>(0.0))), vec3<u32>(3u)), c0);
    // Back to the unmirrored cell order of the mask.
    let lo = select(c0, vec3<u32>(3u) - c1, m);
    let hi = select(c1, vec3<u32>(3u) - c0, m);
    let rx = bit_range4(lo.x, hi.x) * 0x11111111u;
    let ry = ((1u << (4u * hi.y + 4u)) - (1u << (4u * lo.y))) * 0x00010001u;
    let z_lo = select(0u, 0xFFFFu, lo.z == 0u) | select(0u, 0xFFFF0000u, lo.z <= 1u && hi.z >= 1u);
    let z_hi = select(0u, 0xFFFFu, lo.z <= 2u && hi.z >= 2u) | select(0u, 0xFFFF0000u, hi.z == 3u);
    let box_xy = rx & ry;
    return ((mask_lo & box_xy & z_lo) | (mask_hi & box_xy & z_hi)) == 0u;
}

// Levels of the ancestor stack: enough for the world tree (TREE_DEPTH).
const STACK_LEVELS: u32 = 10u;
// Threads per tracing workgroup (8x8).
const STACK_LANES: u32 = 64u;
// The ancestor stack, one row per thread, in workgroup memory because FXC
// cannot write a private array at a runtime index.
var<workgroup> trace_stack: array<u32, 640>;

// The first solid cell along the ray `o + t d` (voxel units, world frame).
// `lane` is the caller's local invocation index (below 64): it picks the
// thread's row of the ancestor stack.
fn cast_ray(o: vec3<f32>, d: vec3<f32>, s: TraceSettings, lane: u32) -> Hit {
    if all(d == vec3<f32>(0.0)) || s.root_level >= STACK_LEVELS || lane >= STACK_LANES {
        return trace_miss(0u, 0u);
    }
    let row = lane * STACK_LEVELS;
    let size = 1u << (2u * (s.root_level + 1u));
    let size_i = i32(size);
    let lo = vec3<f32>(s.origin);
    let hi = lo + vec3<f32>(f32(size));

    // Clip to the tree's box like the reference does. Axes with a zero
    // direction must hold the origin inside their slab.
    let zero = d == vec3<f32>(0.0);
    if any(zero & ((o < lo) | (o > hi))) {
        return trace_miss(0u, 0u);
    }
    let t1 = select((lo - o) / d, vec3<f32>(-T_FAR), zero);
    let t2 = select((hi - o) / d, vec3<f32>(T_FAR), zero);
    let near = select(min(t1, t2), vec3<f32>(-T_FAR), zero);
    let far = select(max(t1, t2), vec3<f32>(T_FAR), zero);
    let t_enter = max(near.x, max(near.y, near.z));
    let t_exit = min(far.x, min(far.y, far.z));
    if t_exit < 0.0 || t_enter > t_exit {
        return trace_miss(0u, 0u);
    }
    // On ties the box is entered by the highest axis: the last one the
    // reference steps at that distance.
    var enter_axis = 0u;
    if near.y >= near.x {
        enter_axis = 1u;
    }
    if near.z >= max(near.x, near.y) {
        enter_axis = 2u;
    }

    let m = d < vec3<f32>(0.0);
    let od = select(o, -o, m);
    let dd = abs(d);
    // The box's lowest voxel in the mirrored frame.
    let mo = select(s.origin, -(s.origin + vec3<i32>(size_i)), m);
    let box_size = vec3<i32>(size_i);
    let fo = vec3<i32>(floor(o));
    var v = select(fo, -fo - vec3<i32>(1), m);
    var t = 0.0;
    var face_axis = 3u;

    let rel0 = v - mo;
    if any(rel0 < vec3<i32>(0)) || any(rel0 >= box_size) {
        // Outside: enter through the plane crossed last.
        v = step_voxel(v, enter_axis, pick_i(mo, enter_axis), od, dd, t_enter);
        t = t_enter;
        face_axis = enter_axis;
    }

    trace_stack[row + s.root_level] = ROOT_NODE;
    var top = s.root_level;
    var steps = 0u;
    let mirror_max = vec3<u32>(size - 1u);
    loop {
        if steps >= s.max_iterations {
            return trace_miss(steps, HIT_FLAG_EXHAUSTED);
        }
        steps += 1u;
        let rel = v - mo;
        if any(rel < vec3<i32>(0)) || any(rel >= box_size) {
            return trace_miss(steps, 0u);
        }
        let ml = vec3<u32>(rel);
        let local = select(ml, mirror_max - ml, m);

        var level = top;
        var index = trace_stack[row + level];
        var node = load_node(index);
        var cell_level = 0u;
        loop {
            let kind = node_kind(node);
            if kind == KIND_UNIFORM {
                return trace_hit(v, m, node.reference & 0xFFFFu, 0u, face_axis, HIT_FLAG_HIT, t, steps);
            }
            let slot = child_slot(local, level);
            if !mask_bit(node.mask_lo, node.mask_hi, slot) {
                cell_level = level;
                if level < s.root_level
                    && rest_of_node_empty(node.mask_lo, node.mask_hi, ml, level, m, mo, od, dd) {
                    cell_level = level + 1u;
                }
                break;
            }
            let cell = f32(1u << (2u * level));
            let coarse_enough = cell < t * s.pixel_angle * s.footprint_bias;
            if kind == KIND_NOT_RESIDENT {
                if !coarse_enough {
                    request_node(index, local, level, t, s);
                }
                return trace_hit(
                    v, m, node_material(node), level, face_axis,
                    HIT_FLAG_HIT | HIT_FLAG_RESIDENCY, t, steps,
                );
            }
            if kind == KIND_BRICK {
                if coarse_enough {
                    return trace_hit(
                        v, m, node_material(node), level, face_axis,
                        HIT_FLAG_HIT | HIT_FLAG_FOOTPRINT, t, steps,
                    );
                }
                let seg = block_segment(node.reference);
                let base = block_base(node.reference);
                stamp_brick(seg, base, s.frame);
                let n = brick_word(seg, base + 1u) & BLOCK_COUNT_MASK;
                let k = mask_rank(node.mask_lo, node.mask_hi, slot);
                let at = base + BLOCK_HEADER_WORDS + k * LEAF_MASK_WORDS;
                let leaf_lo = brick_word(seg, at);
                let leaf_hi = brick_word(seg, at + 1u);
                let vslot = child_slot(local, 0u);
                if !mask_bit(leaf_lo, leaf_hi, vslot) {
                    cell_level = 0u;
                    if rest_of_node_empty(leaf_lo, leaf_hi, ml, 0u, m, mo, od, dd) {
                        cell_level = 1u;
                    }
                    break;
                }
                let material = leaf_material(seg, base, n, k, leaf_lo, leaf_hi, vslot);
                return trace_hit(v, m, material, 0u, face_axis, HIT_FLAG_HIT, t, steps);
            }
            // Internal: one dependent load per level.
            let child = node.reference + mask_rank(node.mask_lo, node.mask_hi, slot);
            if coarse_enough || level <= 1u {
                let c = load_node(child);
                if node_kind(c) == KIND_UNIFORM {
                    return trace_hit(v, m, c.reference & 0xFFFFu, 0u, face_axis, HIT_FLAG_HIT, t, steps);
                }
                return trace_hit(
                    v, m, node_material(c), level, face_axis,
                    HIT_FLAG_HIT | HIT_FLAG_FOOTPRINT, t, steps,
                );
            }
            level -= 1u;
            index = child;
            node = load_node(index);
            trace_stack[row + level] = index;
        }

        // Jump over the empty cell of 4^cell_level voxels containing v.
        let span = 1u << (2u * cell_level);
        let cell_end = (ml | vec3<u32>(span - 1u)) + vec3<u32>(1u);
        let b = vec3<i32>(cell_end) + mo;
        let tn = select(vec3<f32>(T_FAR), (vec3<f32>(b) - od) / dd, dd > vec3<f32>(0.0));
        var k = 2u;
        if tn.x <= tn.y && tn.x <= tn.z {
            k = 0u;
        } else if tn.y <= tn.z {
            k = 1u;
        }
        let tk = pick_f(tn, k);
        if tk > t_exit {
            return trace_miss(steps, 0u);
        }
        let nv = step_voxel(v, k, pick_i(b, k), od, dd, tk);
        // Restart at the deepest node holding both voxels.
        let nrel = nv - mo;
        if any(nrel < vec3<i32>(0)) || any(nrel >= box_size) {
            return trace_miss(steps, 0u);
        }
        let diff = ml ^ vec3<u32>(nrel);
        let any_diff = diff.x | diff.y | diff.z;
        var shared_level = max(cell_level, 1u);
        while shared_level < s.root_level && (any_diff >> (2u * (shared_level + 1u))) != 0u {
            shared_level += 1u;
        }
        top = shared_level;
        v = nv;
        t = tk;
        face_axis = k;
    }
    return trace_miss(steps, 0u);
}
