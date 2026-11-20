//! Frame-time summary for fixed-length runs (`--exit-after-frames`).
//!
//! Real (unclamped) frame times are recorded every frame and summarized when
//! the run ends: once for the loading frames and once for the steady frames
//! after them, so a streaming start does not hide the steady-state cost.

use bevy::prelude::*;

/// Frames up to this one count as loading; the first frame (startup) is
/// skipped entirely.
pub const LOADING_FRAMES: usize = 100;

/// Mean, percentiles and maximum of a set of frame times, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl Summary {
    /// `None` for an empty slice.
    pub fn of(frame_ms: &[f64]) -> Option<Self> {
        if frame_ms.is_empty() {
            return None;
        }
        let mut sorted = frame_ms.to_vec();
        sorted.sort_by(f64::total_cmp);
        // Nearest-rank percentile: the smallest value with at least p of the
        // samples at or below it.
        let rank = |p: f64| {
            let n = sorted.len();
            let i = ((p * n as f64).ceil() as usize).clamp(1, n) - 1;
            sorted[i]
        };
        Some(Self {
            frames: sorted.len(),
            mean_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p50_ms: rank(0.50),
            p95_ms: rank(0.95),
            p99_ms: rank(0.99),
            max_ms: sorted[sorted.len() - 1],
        })
    }

    pub fn fps(&self) -> f64 {
        if self.mean_ms > 0.0 {
            1000.0 / self.mean_ms
        } else {
            0.0
        }
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} frames, mean {:.1} ms ({:.1} fps), p50 {:.1}, p95 {:.1}, p99 {:.1}, max {:.1} ms",
            self.frames,
            self.mean_ms,
            self.fps(),
            self.p50_ms,
            self.p95_ms,
            self.p99_ms,
            self.max_ms
        )
    }
}

/// Splits recorded frame times (index 0 is frame 1) into the loading and
/// steady windows. Frame 1 is dropped: it holds startup.
pub fn windows(frame_ms: &[f64]) -> (&[f64], &[f64]) {
    let rest = frame_ms.get(1..).unwrap_or(&[]);
    let split = rest.len().min(LOADING_FRAMES - 1);
    rest.split_at(split)
}

/// Frame times recorded so far, in milliseconds, and where to write them.
#[derive(Resource, Default)]
pub struct FrameTimes {
    pub ms: Vec<f64>,
    pub csv: Option<std::path::PathBuf>,
}

pub fn record(time: Res<Time<Real>>, mut times: ResMut<FrameTimes>) {
    times.ms.push(time.delta_secs_f64() * 1000.0);
}

/// One frame time per line, frame 1 first.
pub fn to_lines(frame_ms: &[f64]) -> String {
    let mut out = String::with_capacity(frame_ms.len() * 8);
    for ms in frame_ms {
        out.push_str(&format!("{ms:.3}\n"));
    }
    out
}

/// Logs both windows, and writes every frame time when asked to.
pub fn report(times: &FrameTimes) {
    if let Some(path) = &times.csv {
        match std::fs::write(path, to_lines(&times.ms)) {
            Ok(()) => info!("frame times written to {}", path.display()),
            Err(e) => warn!("could not write frame times to {}: {e}", path.display()),
        }
    }
    let (loading, steady) = windows(&times.ms);
    if let Some(s) = Summary::of(loading) {
        info!("frame times, loading (frames 2-{}): {s}", loading.len() + 1);
    }
    if let Some(s) = Summary::of(steady) {
        info!(
            "frame times, steady (frames {}-{}): {s}",
            LOADING_FRAMES + 1,
            LOADING_FRAMES + steady.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_of_nothing_is_none() {
        assert_eq!(Summary::of(&[]), None);
    }

    #[test]
    fn nearest_rank_percentiles() {
        let ms: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = Summary::of(&ms).unwrap();
        assert_eq!(s.frames, 100);
        assert_eq!(s.mean_ms, 50.5);
        assert_eq!(
            (s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms),
            (50.0, 95.0, 99.0, 100.0)
        );
        assert!((s.fps() - 1000.0 / 50.5).abs() < 1e-9);
    }

    #[test]
    fn order_does_not_matter() {
        let a = Summary::of(&[5.0, 1.0, 3.0]).unwrap();
        let b = Summary::of(&[1.0, 3.0, 5.0]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.p50_ms, 3.0);
    }

    #[test]
    fn windows_drop_frame_one_and_split_at_the_loading_limit() {
        let ms: Vec<f64> = (1..=600).map(f64::from).collect();
        let (loading, steady) = windows(&ms);
        assert_eq!(loading.first(), Some(&2.0));
        assert_eq!(loading.last(), Some(&100.0));
        assert_eq!(steady.first(), Some(&101.0));
        assert_eq!(steady.len(), 500);
    }

    #[test]
    fn lines_hold_one_frame_each() {
        assert_eq!(to_lines(&[1.0, 22.125]), "1.000\n22.125\n");
        assert_eq!(to_lines(&[]), "");
    }

    #[test]
    fn short_runs_have_no_steady_window() {
        let (loading, steady) = windows(&[9.0, 1.0, 2.0]);
        assert_eq!(loading, &[1.0, 2.0]);
        assert!(steady.is_empty());
        assert_eq!(windows(&[]), (&[][..], &[][..]));
    }
}
