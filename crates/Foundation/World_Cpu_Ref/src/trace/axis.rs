//! One axis of a ray in the walk's mirrored, cube-local integer space.
//!
//! Voxel `v` on a mirrored axis is original cube-local voxel `size - 1 - v`.
//! Mirrored voxel `u` is entered by crossing mirrored plane `u`, which is the
//! original world plane `origin + u` (moving `+`) or `origin + size - u`
//! (moving `-`). Distances are always computed from the original plane and
//! the original ray, exactly as the DDA reference does.

/// The order of crossings: `(t, axis)` lexicographically, the lower axis
/// first on equal `t` (the DDA steps its lowest axis first on a tie).
#[inline]
pub(super) fn before(t: f32, axis: usize, other_t: f32, other_axis: usize) -> bool {
    t < other_t || (t == other_t && axis < other_axis)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Axis {
    /// 0, 1 or 2.
    index: usize,
    origin: f32,
    dir: f32,
    /// The direction is not zero.
    pub moving: bool,
    /// The direction is negative, so this axis is mirrored.
    negative: bool,
    /// The source cube's lowest voxel on this axis.
    cube_min: i64,
    /// Voxels per axis of the cube.
    size: i64,
}

impl Axis {
    pub fn new(index: usize, origin: f32, dir: f32, cube_min: i64, size: i64) -> Self {
        Self {
            index,
            origin,
            dir,
            moving: dir != 0.0,
            negative: dir < 0.0,
            cube_min,
            size,
        }
    }

    /// Whether moving along this axis goes `+` in world space.
    pub fn positive(&self) -> bool {
        !self.negative
    }

    /// The mirrored voxel of the ray origin: `floor(origin)` exactly as the
    /// DDA computes it (a saturating `as i32`), made cube-local and mirrored.
    pub fn start_voxel(&self) -> i64 {
        let local = i64::from(self.origin.floor() as i32) - self.cube_min;
        self.mirror(local)
    }

    /// Mirrored to original cube-local coordinate, and back (an involution).
    pub fn mirror(&self, v: i64) -> i64 {
        if self.negative { self.size - 1 - v } else { v }
    }

    /// The world voxel coordinate of mirrored voxel `v`.
    pub fn world(&self, v: i64) -> i64 {
        self.cube_min + self.mirror(v)
    }

    /// The distance at which the ray enters mirrored voxel `u`, i.e. crosses
    /// mirrored plane `u`: `(plane as f32 - origin) / dir` with the plane in
    /// original world coordinates. Only meaningful on a moving axis.
    #[inline]
    pub fn crossing(&self, u: i64) -> f32 {
        let plane = if self.negative {
            self.cube_min + self.size - u
        } else {
            self.cube_min + u
        };
        (plane as f32 - self.origin) / self.dir
    }

    /// The voxel this axis is in once the crossing `(t, axis)` has happened:
    /// the largest `u` in `from..=last` whose entry crossing is ordered
    /// before `(t, axis)`, or `from` if none is. Crossing distances are
    /// non-decreasing in `u` (monotone rounding), so the predicate is true
    /// then false and a search finds the boundary exactly. A float estimate
    /// is tried first and confirmed with the exact predicate; a binary search
    /// takes over when it is off.
    pub fn voxel_at(&self, from: i64, last: i64, t: f32, axis: usize) -> i64 {
        debug_assert!(self.moving && from <= last && axis != self.index);
        let entered = |u: i64| before(self.crossing(u), self.index, t, axis);
        let guess = self.estimate(t).clamp(from, last);
        let guess_ok = guess == from || entered(guess);
        let next_ok = guess == last || !entered(guess + 1);
        if guess_ok && next_ok {
            return guess;
        }
        let (mut lo, mut hi) = (from, last);
        while lo < hi {
            let mid = lo + (hi - lo + 1) / 2;
            if entered(mid) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }

    /// Roughly the mirrored voxel at distance `t`, from `origin + dir * t`.
    /// Only a starting guess; saturates on huge or infinite values.
    fn estimate(&self, t: f32) -> i64 {
        let p = self.origin + self.dir * t;
        let local = (p.floor() as i64).saturating_sub(self.cube_min);
        if self.negative {
            (self.size - 1).saturating_sub(local)
        } else {
            local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossings_match_the_dda_formula_in_both_directions() {
        // Cube [0, 16). Moving +: mirrored voxel 5 is entered at plane 5.
        let a = Axis::new(0, 2.25, 0.5, 0, 16);
        assert_eq!(a.start_voxel(), 2);
        assert_eq!(a.crossing(5), (5.0 - 2.25) / 0.5);
        // Moving -: mirrored voxel u is original voxel 15 - u, entered at
        // original plane 16 - u.
        let b = Axis::new(1, 7.5, -0.25, 0, 16);
        assert_eq!(b.start_voxel(), 8);
        assert_eq!(b.world(8), 7);
        assert_eq!(b.crossing(9), (7.0 - 7.5) / -0.25);
    }

    #[test]
    fn voxel_at_counts_crossings_before_the_event() {
        let a = Axis::new(1, 0.5, 1.0, 0, 64);
        // Crossing into u happens at t = u - 0.5.
        assert_eq!(a.voxel_at(0, 63, 3.5, 0), 3, "x steps before y on a tie");
        assert_eq!(a.voxel_at(0, 63, 3.5, 2), 4, "y steps before z on a tie");
        assert_eq!(a.voxel_at(0, 63, 3.4, 2), 3);
        assert_eq!(a.voxel_at(0, 10, 100.0, 2), 10, "clamped to the cell");
        assert_eq!(a.voxel_at(2, 63, 0.1, 2), 2, "never below the start");
        assert_eq!(a.voxel_at(0, 63, f32::INFINITY, 2), 63);
    }
}
