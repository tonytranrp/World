//! What a test does when a prerequisite (such as a GPU adapter) is missing.
//!
//! Locally the test prints a loud `SKIPPED` line and passes. With
//! `WORLD_TEST_REQUIRE_GPU=1`, which CI sets, the same situation fails the
//! test, so a skipped suite can never look green on the build server.

pub const REQUIRE_ENV: &str = "WORLD_TEST_REQUIRE_GPU";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Print the message and let the test pass.
    Skip(String),
    /// Fail the test with the message.
    Fail(String),
}

/// Pure decision, separated from the environment so it can be tested.
pub fn decide(required: bool, reason: &str) -> Outcome {
    if required {
        Outcome::Fail(format!(
            "required prerequisite missing: {reason} ({REQUIRE_ENV}=1 forbids skipping)"
        ))
    } else {
        Outcome::Skip(format!("SKIPPED: {reason}"))
    }
}

/// Whether skipping is forbidden in this environment.
pub fn required_from_env() -> bool {
    matches!(std::env::var(REQUIRE_ENV).as_deref(), Ok("1"))
}

/// Call when a prerequisite is missing, then `return` from the test.
/// Panics when the environment requires the prerequisite.
pub fn skip_or_fail(reason: &str) {
    match decide(required_from_env(), reason) {
        Outcome::Skip(message) => eprintln!("{message}"),
        Outcome::Fail(message) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_required_skips_with_a_loud_prefix() {
        let outcome = decide(false, "no software adapter");
        assert_eq!(
            outcome,
            Outcome::Skip("SKIPPED: no software adapter".into())
        );
    }

    #[test]
    fn required_fails_and_names_the_variable() {
        match decide(true, "no software adapter") {
            Outcome::Fail(m) => {
                assert!(m.contains(REQUIRE_ENV) && m.contains("no software adapter"))
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }
}
