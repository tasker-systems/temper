//! Output format selector for CLI commands.
//!
//! Strict policy: this module is the **only** place the `toon-format` crate
//! is imported. Callers receive `String` from `render` / `render_resource_show`
//! and never touch toon types directly. Swapping the Toon backend (to a
//! competing crate or a hand-rolled implementation) touches this file only.

use std::io::IsTerminal;

use serde::Serialize;
use temper_core::error::TemperError;

/// CLI output format. Two formats only: `Json` (strict wire-type passthrough
/// of cloud API responses) and `Toon` (human-readable rendering of the same
/// data via the `toon-format` crate, TOON v3.0 spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Toon,
}

impl OutputFormat {
    /// Parse a `--format` string. Unknown values auto-detect via TTY.
    pub fn parse(s: &str) -> Self {
        Self::parse_opt(s).unwrap_or_else(Self::auto)
    }

    /// Parse a format string, returning `None` for unrecognized values.
    ///
    /// Used by `resolve_with` so that unknown env/config values fall through
    /// to the next precedence layer rather than locking to the tty default.
    fn parse_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "json" => Some(Self::Json),
            "toon" => Some(Self::Toon),
            _ => None,
        }
    }

    /// Resolve the effective format given an optional explicit CLI value.
    pub fn resolve(explicit: Option<&str>) -> Self {
        match explicit {
            Some(s) => Self::parse(s),
            None => Self::auto(),
        }
    }

    /// Resolve the effective format with full precedence (highest to lowest):
    /// 1. `explicit` CLI flag value
    /// 2. `TEMPER_FORMAT` environment variable
    /// 3. `config_default` (from the `[cli].format` config key)
    /// 4. TTY-aware default (Toon on a terminal stdout, Json otherwise)
    ///
    /// At each of layers 1–3 an *unrecognized* value falls through to the next
    /// layer rather than forcing a default, so a typo in env/config degrades to
    /// the tty-aware default instead of silently locking a format.
    pub fn resolve_with(explicit: Option<&str>, config_default: Option<&str>) -> Self {
        // Layer 1: explicit CLI flag
        if let Some(s) = explicit {
            if let Some(fmt) = Self::parse_opt(s) {
                return fmt;
            }
        }
        // Layer 2: TEMPER_FORMAT env var (empty string treated as unset)
        if let Ok(v) = std::env::var("TEMPER_FORMAT") {
            if !v.is_empty() {
                if let Some(fmt) = Self::parse_opt(&v) {
                    return fmt;
                }
            }
        }
        // Layer 3: config file default ([cli].format)
        if let Some(s) = config_default {
            if let Some(fmt) = Self::parse_opt(s) {
                return fmt;
            }
        }
        // Layer 4: TTY-aware default
        Self::auto()
    }

    /// Auto-pick based on whether stdout is a terminal: TTY → Toon, else Json.
    fn auto() -> Self {
        if std::io::stdout().is_terminal() {
            Self::Toon
        } else {
            Self::Json
        }
    }

    /// Canonical string form for the few remaining `&str`-taking callsites.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toon => "toon",
        }
    }
}

/// Render any `Serialize` value in the chosen format. `Json` uses
/// `serde_json::to_string_pretty`; `Toon` uses `toon_format::encode_default`,
/// which accepts `T: Serialize` directly.
pub fn render<T: Serialize>(value: &T, fmt: OutputFormat) -> Result<String, TemperError> {
    match fmt {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
        OutputFormat::Toon => toon_format::encode_default(value)
            .map_err(|e| TemperError::Api(format!("toon render: {e}"))),
    }
}

/// `temper resource show` exception: Toon emits markdown body with the
/// frontmatter at the top; Json emits a composite shape
/// `{ ...metadata, content: "<body>" }`.
pub fn render_resource_show(
    metadata: &serde_json::Value,
    body: &str,
    fmt: OutputFormat,
) -> Result<String, TemperError> {
    match fmt {
        OutputFormat::Toon => {
            let frontmatter = serde_yaml::to_string(metadata)?;
            Ok(format!("---\n{frontmatter}---\n{body}"))
        }
        OutputFormat::Json => {
            let mut composite = metadata.clone();
            if let Some(obj) = composite.as_object_mut() {
                obj.insert(
                    "content".to_string(),
                    serde_json::Value::String(body.to_string()),
                );
            }
            Ok(serde_json::to_string_pretty(&composite)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    fn parse_json_lowercase() {
        assert_eq!(OutputFormat::parse("json"), OutputFormat::Json);
    }

    #[test]
    fn parse_toon_lowercase() {
        assert_eq!(OutputFormat::parse("toon"), OutputFormat::Toon);
    }

    #[test]
    fn parse_unknown_defaults_to_auto() {
        let v = OutputFormat::parse("text");
        assert!(matches!(v, OutputFormat::Toon | OutputFormat::Json));
    }

    #[test]
    fn resolve_explicit_honors_value() {
        assert_eq!(OutputFormat::resolve(Some("json")), OutputFormat::Json);
        assert_eq!(OutputFormat::resolve(Some("toon")), OutputFormat::Toon);
    }

    #[derive(Serialize)]
    struct Fixture {
        slug: &'static str,
        score: f32,
    }

    #[test]
    fn render_json_emits_serde_json_pretty() {
        let f = Fixture {
            slug: "hello",
            score: 0.5,
        };
        let out = render(&f, OutputFormat::Json).expect("json render");
        assert!(out.contains("\"slug\": \"hello\""), "json: {out}");
        assert!(out.contains("\"score\": 0.5"), "json: {out}");
    }

    #[test]
    fn render_toon_emits_key_and_value() {
        let f = Fixture {
            slug: "hello",
            score: 0.5,
        };
        let out = render(&f, OutputFormat::Toon).expect("toon render");
        assert!(out.contains("slug"), "toon: {out}");
        assert!(out.contains("hello"), "toon: {out}");
    }

    // -- resolve_with precedence tests --

    /// Layer 1 (explicit flag) wins over env and config.
    #[test]
    fn resolve_with_explicit_wins_over_env_and_config() {
        temp_env::with_var("TEMPER_FORMAT", Some("toon"), || {
            assert_eq!(
                OutputFormat::resolve_with(Some("json"), Some("toon")),
                OutputFormat::Json
            );
        });
    }

    /// Layer 2 (env) wins over config when no flag is given.
    #[test]
    fn resolve_with_env_wins_over_config() {
        temp_env::with_var("TEMPER_FORMAT", Some("json"), || {
            assert_eq!(
                OutputFormat::resolve_with(None, Some("toon")),
                OutputFormat::Json
            );
        });
    }

    /// Layer 3 (config) used when no flag and env is unset.
    #[test]
    fn resolve_with_config_used_when_no_flag_and_env_unset() {
        temp_env::with_var_unset("TEMPER_FORMAT", || {
            assert_eq!(
                OutputFormat::resolve_with(None, Some("json")),
                OutputFormat::Json
            );
        });
    }

    /// Unrecognized env value falls through to config.
    #[test]
    fn resolve_with_unrecognized_env_falls_through_to_config() {
        temp_env::with_var("TEMPER_FORMAT", Some("garbage"), || {
            assert_eq!(
                OutputFormat::resolve_with(None, Some("toon")),
                OutputFormat::Toon
            );
        });
    }

    /// Unrecognized config with no flag/env falls through to tty-aware default.
    #[test]
    fn resolve_with_unrecognized_config_falls_through_to_auto() {
        temp_env::with_var_unset("TEMPER_FORMAT", || {
            let v = OutputFormat::resolve_with(None, Some("garbage"));
            // Can't assert a fixed variant: TTY detection is environment-dependent.
            assert!(matches!(v, OutputFormat::Json | OutputFormat::Toon));
        });
    }

    /// Empty-string env is treated as unset; config layer applies instead.
    #[test]
    fn resolve_with_empty_env_treated_as_unset() {
        temp_env::with_var("TEMPER_FORMAT", Some(""), || {
            assert_eq!(
                OutputFormat::resolve_with(None, Some("json")),
                OutputFormat::Json
            );
        });
    }

    /// parse_opt returns None for unrecognized strings.
    #[test]
    fn parse_opt_unknown_returns_none() {
        assert_eq!(OutputFormat::parse_opt("text"), None);
        assert_eq!(OutputFormat::parse_opt(""), None);
    }

    /// parse_opt returns Some for recognized strings (case-insensitive).
    #[test]
    fn parse_opt_recognized_returns_some() {
        assert_eq!(OutputFormat::parse_opt("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse_opt("JSON"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse_opt("toon"), Some(OutputFormat::Toon));
        assert_eq!(OutputFormat::parse_opt("TOON"), Some(OutputFormat::Toon));
    }
}
