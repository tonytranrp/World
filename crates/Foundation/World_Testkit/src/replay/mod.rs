//! Deterministic replays: a scenario run step by step, one state hash per step.
//!
//! A replay proves two things: a scenario run twice gives identical hashes
//! (determinism), and a run today equals a recorded run (regression).

use std::fmt;
use std::path::Path;

use crate::golden;

const HEADER: &str = "replay v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    pub scenario: String,
    /// State hash after each step.
    pub hashes: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayDifference {
    Scenario {
        expected: String,
        actual: String,
    },
    Length {
        expected: usize,
        actual: usize,
    },
    Step {
        step: usize,
        expected: u64,
        actual: u64,
    },
}

impl fmt::Display for ReplayDifference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scenario { expected, actual } => {
                write!(f, "scenario is {actual:?}, expected {expected:?}")
            }
            Self::Length { expected, actual } => {
                write!(f, "ran {actual} steps, expected {expected}")
            }
            Self::Step {
                step,
                expected,
                actual,
            } => write!(
                f,
                "state diverged at step {step}: hash {actual:016x}, expected {expected:016x}"
            ),
        }
    }
}

impl Replay {
    /// Runs `steps` steps. `step(i)` advances the simulation and returns the
    /// state hash after step `i`.
    pub fn record(scenario: &str, steps: usize, mut step: impl FnMut(usize) -> u64) -> Self {
        Self {
            scenario: scenario.to_owned(),
            hashes: (0..steps).map(&mut step).collect(),
        }
    }

    /// Stable text form: a header, the scenario, the step count, one hash per line.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "{HEADER}\nscenario {}\nsteps {}\n",
            self.scenario,
            self.hashes.len()
        );
        for h in &self.hashes {
            out.push_str(&format!("{h:016x}\n"));
        }
        out
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(HEADER) {
            return Err(format!("not a replay: first line must be {HEADER:?}"));
        }
        let scenario = lines
            .next()
            .and_then(|l| l.strip_prefix("scenario "))
            .ok_or("missing `scenario` line")?
            .trim()
            .to_owned();
        let steps: usize = lines
            .next()
            .and_then(|l| l.strip_prefix("steps "))
            .ok_or("missing `steps` line")?
            .trim()
            .parse()
            .map_err(|e| format!("bad step count: {e}"))?;
        let hashes = lines
            .map(|l| u64::from_str_radix(l.trim(), 16).map_err(|e| format!("bad hash {l:?}: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        if hashes.len() != steps {
            return Err(format!(
                "header says {steps} steps but {} hashes follow",
                hashes.len()
            ));
        }
        Ok(Self { scenario, hashes })
    }

    /// The first way `actual` differs from `self` (the expected replay).
    pub fn first_difference(&self, actual: &Replay) -> Option<ReplayDifference> {
        if self.scenario != actual.scenario {
            return Some(ReplayDifference::Scenario {
                expected: self.scenario.clone(),
                actual: actual.scenario.clone(),
            });
        }
        if let Some((step, (e, a))) = self
            .hashes
            .iter()
            .zip(&actual.hashes)
            .enumerate()
            .find(|(_, (e, a))| e != a)
        {
            return Some(ReplayDifference::Step {
                step,
                expected: *e,
                actual: *a,
            });
        }
        (self.hashes.len() != actual.hashes.len()).then_some(ReplayDifference::Length {
            expected: self.hashes.len(),
            actual: actual.hashes.len(),
        })
    }
}

/// Runs a scenario twice from scratch and requires identical hashes at every step.
/// `make` builds a fresh run each time, so no state can leak between the two.
pub fn assert_deterministic<S: FnMut(usize) -> u64>(
    scenario: &str,
    steps: usize,
    make: impl Fn() -> S,
) {
    let first = Replay::record(scenario, steps, make());
    let second = Replay::record(scenario, steps, make());
    if let Some(difference) = first.first_difference(&second) {
        panic!("scenario {scenario:?} is not deterministic: {difference}");
    }
}

/// Asserts that `actual` equals the recorded replay at `path`. With
/// `UPDATE_GOLDEN=1` the recording is rewritten instead.
pub fn assert_replay_golden(path: &Path, actual: &Replay) {
    if golden::update_requested() {
        golden::assert_text_golden(path, &actual.to_text());
        return;
    }
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "recorded replay {} does not exist. Check the run is right, then record it with \
             `cargo xtask golden --update`.",
            path.display()
        )
    });
    let expected = Replay::from_text(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if let Some(difference) = expected.first_difference(actual) {
        panic!("replay differs from {}: {difference}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Replay {
        Replay {
            scenario: "tower collapse".into(),
            hashes: vec![1, 0xDEAD_BEEF, u64::MAX],
        }
    }

    #[test]
    fn text_round_trip() {
        let r = sample();
        assert_eq!(Replay::from_text(&r.to_text()), Ok(r));
    }

    #[test]
    fn malformed_text_is_rejected_with_a_reason() {
        assert!(
            Replay::from_text("hello")
                .unwrap_err()
                .contains("not a replay")
        );
        assert!(
            Replay::from_text("replay v1\nscenario x\nsteps 2\n00\n")
                .unwrap_err()
                .contains("2 steps")
        );
    }

    #[test]
    fn step_divergence_names_the_step() {
        let mut other = sample();
        other.hashes[1] ^= 1;
        assert!(matches!(
            sample().first_difference(&other),
            Some(ReplayDifference::Step { step: 1, .. })
        ));
    }

    #[test]
    fn length_difference_is_found_after_matching_prefix() {
        let mut other = sample();
        other.hashes.pop();
        assert!(matches!(
            sample().first_difference(&other),
            Some(ReplayDifference::Length { .. })
        ));
    }

    #[test]
    fn equal_replays_have_no_difference() {
        assert_eq!(sample().first_difference(&sample()), None);
    }

    #[test]
    fn a_pure_scenario_is_deterministic() {
        assert_deterministic("counter", 50, || {
            let mut state = 0u64;
            move |i| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(i as u64);
                state
            }
        });
    }

    #[test]
    #[should_panic(expected = "not deterministic")]
    fn a_scenario_reading_hidden_state_is_caught() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static HIDDEN: AtomicU64 = AtomicU64::new(0);
        assert_deterministic("leaky", 5, || {
            let offset = HIDDEN.fetch_add(1, Ordering::SeqCst);
            move |i| i as u64 + offset
        });
    }
}
