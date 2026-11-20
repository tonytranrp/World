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
        face = 2u * face_axis + select(0u, 1u, m[face_axis]);
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

// The first solid cell along the ray `o + t d` (voxel units, world frame).
fn cast_ray(o: vec3<f32>, d: vec3<f32>, s: TraceSettings) -> Hit {
    if all(d == vec3<f32>(0.0)) {
        return trace_miss(0u, 0u);
    }
    let size = 1u << (2u * (s.root_level + 1u));
    let size_i = i32(size);
    let lo = vec3<f32>(s.origin);
    let hi = lo + vec3<f32>(f32(size));

    // Clip to the tree's box like the reference does.
    var t_enter = -T_FAR;
    var t_exit = T_FAR;
    var enter_axis = 0u;
    for (var a = 0u; a < 3u; a++) {
        if d[a] == 0.0 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return trace_miss(0u, 0u);
            }
        } else {
            let t1 = (lo[a] - o[a]) / d[a];
            let t2 = (hi[a] - o[a]) / d[a];
            let near = min(t1, t2);
            // `>=`: on ties the box is entered by the highest axis, the
            // last one the reference steps at that distance.
            if near >= t_enter {
                t_enter = near;
                enter_axis = a;
            }
            t_exit = min(t_exit, max(t1, t2));
        }
    }
    if t_exit < 0.0 || t_enter > t_exit {
        return trace_miss(0u, 0u);
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
        let k = enter_axis;
        var nv = v;
        for (var a = 0u; a < 3u; a++) {
            if a == k {
                nv[a] = mo[a];
            } else {
                nv[a] = axis_voxel(v[a], od[a], dd[a], t_enter, a < k);
            }
        }
        v = nv;
        t = t_enter;
        face_axis = k;
    }

    var stack: array<u32, 12>;
    stack[s.root_level] = ROOT_NODE;
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
        var index = stack[level];
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
                    break;
                }
                let material = leaf_material(seg, base, n, k, leaf_lo, leaf_hi, vslot);
                return trace_hit(v, m, material, 0u, face_axis, HIT_FLAG_HIT, t, steps);
            }
            // Internal: one dependent load per level.
            let child = node.reference + mask_rank(node.mask_lo, node.mask_hi, slot);
            if coarse_enough || level == 0u {
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
            stack[level] = index;
        }

        // Jump over the empty cell of 4^cell_level voxels containing v.
        let span = 1u << (2u * cell_level);
        let cell_end = (ml | vec3<u32>(span - 1u)) + vec3<u32>(1u);
        let b = vec3<i32>(cell_end) + mo;
        var tn = vec3<f32>(T_FAR);
        for (var a = 0u; a < 3u; a++) {
            if dd[a] > 0.0 {
                tn[a] = (f32(b[a]) - od[a]) / dd[a];
            }
        }
        var k = 2u;
        if tn.x <= tn.y && tn.x <= tn.z {
            k = 0u;
        } else if tn.y <= tn.z {
            k = 1u;
        }
        let tk = tn[k];
        if tk > t_exit {
            return trace_miss(steps, 0u);
        }
        var nv = v;
        for (var a = 0u; a < 3u; a++) {
            if a == k {
                nv[a] = b[a];
            } else {
                nv[a] = axis_voxel(v[a], od[a], dd[a], tk, a < k);
            }
        }
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
