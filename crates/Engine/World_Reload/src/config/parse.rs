//! Text to typed config and back. Pure functions, so they test without Bevy.

use std::fmt;

use super::ConfigAsset;

/// Why a config text was rejected. Nothing is applied when this is returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The bytes are not UTF-8.
    NotUtf8,
    /// RON syntax, a wrong type, an unknown field, or a check done while parsing.
    Parse {
        line: usize,
        column: usize,
        message: String,
    },
    /// The text parsed but [`ConfigAsset::validate`] rejected the values.
    Invalid(Vec<String>),
    /// The file could not be read.
    Io(String),
    /// No config with this name is registered.
    UnknownConfig(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 => f.write_str("the file is not valid UTF-8"),
            Self::Parse {
                line,
                column,
                message,
            } => write!(f, "line {line}, column {column}: {message}"),
            Self::Invalid(problems) => write!(f, "invalid values: {}", problems.join("; ")),
            Self::Io(e) => write!(f, "could not read the file: {e}"),
            Self::UnknownConfig(name) => write!(f, "no config is named {name:?}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Parses and validates. The only way text becomes a live config. A leading
/// byte-order mark, which some Windows editors save, is ignored.
pub fn parse_config<T: ConfigAsset>(text: &str) -> Result<T, ConfigError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: T = ron::Options::default()
        .from_str(text)
        .map_err(|e| ConfigError::Parse {
            line: e.span.start.line,
            column: e.span.start.col,
            message: e.code.to_string(),
        })?;
    value.validate().map_err(ConfigError::Invalid)?;
    Ok(value)
}

/// Parses raw bytes, as the asset loader receives them.
pub fn parse_config_bytes<T: ConfigAsset>(bytes: &[u8]) -> Result<T, ConfigError> {
    parse_config(std::str::from_utf8(bytes).map_err(|_| ConfigError::NotUtf8)?)
}

/// Pretty RON for display and saving. Field order follows the type.
pub fn to_pretty_ron<T: ConfigAsset>(value: &T) -> String {
    let config = ron::ser::PrettyConfig::new()
        .depth_limit(8)
        .struct_names(false)
        .indentor("    ".to_string());
    ron::ser::to_string_pretty(value, config).expect("config types always serialize")
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Speed {
        metres_per_second: f32,
    }

    impl ConfigAsset for Speed {
        const NAME: &'static str = "speed";
        const PATH: &'static str = "config/speed.ron";
        fn validate(&self) -> Result<(), Vec<String>> {
            if self.metres_per_second < 0.0 {
                Err(vec!["metres_per_second must not be negative".into()])
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn parses_a_valid_config() {
        let s: Speed = parse_config("(metres_per_second: 4.5)").unwrap();
        assert_eq!(s.metres_per_second, 4.5);
    }

    #[test]
    fn syntax_errors_carry_a_position() {
        let err = parse_config::<Speed>("(\n  metres_per_second: )").unwrap_err();
        match err {
            ConfigError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = parse_config::<Speed>("(metres_per_second: 1.0, typo: 2.0)").unwrap_err();
        assert!(err.to_string().contains("typo"), "{err}");
    }

    #[test]
    fn validation_runs_after_parsing() {
        let err = parse_config::<Speed>("(metres_per_second: -1.0)").unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid(vec!["metres_per_second must not be negative".into()])
        );
    }

    #[test]
    fn bytes_must_be_utf8_and_may_start_with_a_bom() {
        assert_eq!(
            parse_config_bytes::<Speed>(&[0xff, 0xfe]).unwrap_err(),
            ConfigError::NotUtf8
        );
        let with_bom = "\u{feff}(metres_per_second: 2.0)";
        assert_eq!(
            parse_config_bytes::<Speed>(with_bom.as_bytes()).unwrap(),
            Speed {
                metres_per_second: 2.0
            }
        );
    }

    #[test]
    fn text_with_a_bom_parses_too() {
        let s: Speed = parse_config("\u{feff}(metres_per_second: 3.0)").unwrap();
        assert_eq!(s.metres_per_second, 3.0);
    }

    #[test]
    fn pretty_output_parses_back_to_the_same_value() {
        let s = Speed {
            metres_per_second: 7.25,
        };
        assert_eq!(parse_config::<Speed>(&to_pretty_ron(&s)).unwrap(), s);
    }
}
