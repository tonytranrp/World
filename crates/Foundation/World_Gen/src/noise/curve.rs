//! Small curves in plain `f32`: smoothstep, piecewise-linear maps and
//! bilinear interpolation, each with a documented order of operations so the
//! result is the same on every machine.

/// Hermite smoothstep from `edge0` to `edge1` (`edge0 < edge1`): 0 below,
/// 1 above, `t²(3 - 2t)` between, with `t = (x - edge0) / (edge1 - edge0)`
/// clamped to `[0, 1]`.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear interpolation of a cell's corners `v00` (low x, low z), `v10`,
/// `v01` and `v11` at fractions `fx`, `fz` in `[0, 1]`. Order: interpolate
/// along x on the low-z edge, then on the high-z edge, then along z, each as
/// `a + (b - a) * f`.
#[inline]
pub fn bilerp(v00: f32, v10: f32, v01: f32, v11: f32, fx: f32, fz: f32) -> f32 {
    let low = v00 + (v10 - v00) * fx;
    let high = v01 + (v11 - v01) * fx;
    low + (high - low) * fz
}

/// A piecewise-linear map through points with strictly increasing `x`,
/// constant beyond the first and last point. Built only from validated points
/// (see [`PiecewiseLinear::new`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PiecewiseLinear {
    points: Vec<[f32; 2]>,
}

impl PiecewiseLinear {
    /// The map through `points`, or every problem with them: fewer than two
    /// points, a non-finite value, or `x` not strictly increasing.
    pub fn new(points: &[[f32; 2]]) -> Result<Self, Vec<String>> {
        let problems = Self::problems(points);
        if problems.is_empty() {
            Ok(Self {
                points: points.to_vec(),
            })
        } else {
            Err(problems)
        }
    }

    /// Every problem with `points` as a curve.
    pub fn problems(points: &[[f32; 2]]) -> Vec<String> {
        let mut out = Vec::new();
        if points.len() < 2 {
            out.push(format!("needs at least 2 points, has {}", points.len()));
        }
        for (i, p) in points.iter().enumerate() {
            if !p[0].is_finite() || !p[1].is_finite() {
                out.push(format!("point {i} {p:?} is not finite"));
            }
        }
        for (i, w) in points.windows(2).enumerate() {
            if w[0][0] >= w[1][0] {
                out.push(format!(
                    "point {} x {} is not above point {i} x {}",
                    i + 1,
                    w[1][0],
                    w[0][0]
                ));
            }
        }
        out
    }

    /// The value at `x`: `y0 + (y1 - y0) * ((x - x0) / (x1 - x0))` on the
    /// segment containing `x`, found by a linear scan in point order.
    pub fn eval(&self, x: f32) -> f32 {
        let first = self.points[0];
        if x <= first[0] {
            return first[1];
        }
        for w in self.points.windows(2) {
            let ([x0, y0], [x1, y1]) = (w[0], w[1]);
            if x <= x1 {
                return y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
            }
        }
        self.points[self.points.len() - 1][1]
    }

    /// The smallest and largest value the map can return.
    pub fn range(&self) -> (f32, f32) {
        self.points
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p[1]), hi.max(p[1]))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothstep_clamps_and_is_symmetric() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert_eq!(smoothstep(0.0, 1.0, 0.5), 0.5);
    }

    #[test]
    fn bilerp_hits_its_corners() {
        assert_eq!(bilerp(1.0, 2.0, 3.0, 4.0, 0.0, 0.0), 1.0);
        assert_eq!(bilerp(1.0, 2.0, 3.0, 4.0, 1.0, 0.0), 2.0);
        assert_eq!(bilerp(1.0, 2.0, 3.0, 4.0, 0.0, 1.0), 3.0);
        assert_eq!(bilerp(1.0, 2.0, 3.0, 4.0, 1.0, 1.0), 4.0);
        assert_eq!(bilerp(1.0, 2.0, 3.0, 4.0, 0.5, 0.5), 2.5);
    }

    #[test]
    fn piecewise_linear_interpolates_and_clamps() {
        let c = PiecewiseLinear::new(&[[-1.0, -10.0], [0.0, 0.0], [1.0, 30.0]]).unwrap();
        assert_eq!(c.eval(-5.0), -10.0);
        assert_eq!(c.eval(-0.5), -5.0);
        assert_eq!(c.eval(0.5), 15.0);
        assert_eq!(c.eval(9.0), 30.0);
        assert_eq!(c.range(), (-10.0, 30.0));
    }

    #[test]
    fn bad_curves_report_every_problem() {
        let err = PiecewiseLinear::new(&[[0.0, f32::NAN], [0.0, 1.0]]).unwrap_err();
        assert_eq!(err.len(), 2, "{err:?}");
        assert!(PiecewiseLinear::new(&[[0.0, 1.0]]).is_err());
    }
}
