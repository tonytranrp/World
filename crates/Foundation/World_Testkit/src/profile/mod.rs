//! Test scale: a fast profile for every commit, a full profile for the plan's
//! one-million-edit criterion.

use proptest::test_runner::Config as ProptestConfig;

/// Environment variable selecting the profile: `fast` (default) or `full`.
pub const PROFILE_ENV: &str = "WORLD_TEST_PROFILE";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Fast,
    Full,
}

/// How big a property test should be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    /// Number of random cases per property test.
    pub cases: u32,
    /// Random edits applied within each case.
    pub edits_per_case: usize,
}

impl Profile {
    /// Parses a profile name. `None` means "not set", which is the fast profile.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim) {
            None | Some("") | Some("fast") => Ok(Self::Fast),
            Some("full") => Ok(Self::Full),
            Some(other) => Err(format!(
                "{PROFILE_ENV}={other:?} is not a profile; use \"fast\" or \"full\""
            )),
        }
    }

    /// The profile selected by the environment. Panics on an unknown value so a
    /// typo can never silently run the wrong suite.
    pub fn from_env() -> Self {
        Self::parse(std::env::var(PROFILE_ENV).ok().as_deref()).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fast: 256 cases of 32 edits. Full: 1,000 cases of 1,000 edits, which is
    /// the plan's 1,000,000 random edits.
    pub const fn scale(self) -> Scale {
        match self {
            Self::Fast => Scale {
                cases: 256,
                edits_per_case: 32,
            },
            Self::Full => Scale {
                cases: 1_000,
                edits_per_case: 1_000,
            },
        }
    }
}

/// Applies a `PROPTEST_CASES` override to a scale. A value that is not a
/// positive number is an error, so a typo cannot silently change the suite.
pub fn with_cases_override(scale: Scale, value: Option<&str>) -> Result<Scale, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(scale),
        Some(text) => match text.parse::<u32>() {
            Ok(cases) if cases > 0 => Ok(Scale { cases, ..scale }),
            _ => Err(format!("PROPTEST_CASES={text:?} is not a positive number")),
        },
    }
}

/// The scale of the active profile. `PROPTEST_CASES`, when set, overrides the
/// number of cases (used to keep slow runs such as Miri short).
pub fn scale() -> Scale {
    with_cases_override(
        Profile::from_env().scale(),
        std::env::var("PROPTEST_CASES").ok().as_deref(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// A proptest configuration sized by the active profile.
pub fn proptest_config() -> ProptestConfig {
    ProptestConfig::with_cases(scale().cases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_and_empty_mean_fast() {
        assert_eq!(Profile::parse(None), Ok(Profile::Fast));
        assert_eq!(Profile::parse(Some("")), Ok(Profile::Fast));
        assert_eq!(Profile::parse(Some(" fast ")), Ok(Profile::Fast));
    }

    #[test]
    fn cases_override_changes_only_the_case_count() {
        let base = Profile::Full.scale();
        let tuned = with_cases_override(base, Some("4")).unwrap();
        assert_eq!(tuned.cases, 4);
        assert_eq!(tuned.edits_per_case, base.edits_per_case);
        assert_eq!(with_cases_override(base, None), Ok(base));
    }

    #[test]
    fn bad_cases_override_is_an_error() {
        for bad in ["0", "-3", "many"] {
            assert!(
                with_cases_override(Profile::Fast.scale(), Some(bad)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn full_is_recognised() {
        assert_eq!(Profile::parse(Some("full")), Ok(Profile::Full));
    }

    #[test]
    fn unknown_value_is_an_error_naming_the_variable() {
        let err = Profile::parse(Some("fats")).unwrap_err();
        assert!(err.contains(PROFILE_ENV) && err.contains("fats"));
    }

    #[test]
    fn full_profile_is_one_million_edits() {
        let s = Profile::Full.scale();
        assert_eq!(s.cases as usize * s.edits_per_case, 1_000_000);
    }

    #[test]
    fn fast_is_much_smaller_than_full() {
        let (f, g) = (Profile::Fast.scale(), Profile::Full.scale());
        assert!((f.cases as usize * f.edits_per_case) * 100 < g.cases as usize * g.edits_per_case);
    }
}
