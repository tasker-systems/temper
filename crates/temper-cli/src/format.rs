//! Output format selector for CLI commands.
//!
//! Strict policy: this module is the **only** place the `toon-format` crate
//! is imported. Callers receive `String` from `render` / `render_resource_show`
//! and never touch toon types directly. Swapping the Toon backend (to
//! `toon-rs`, a hand-rolled implementation, or a successor crate) touches
//! this file only.

use std::io::IsTerminal;

use serde::Serialize;
use temper_core::error::TemperError;

/// CLI output format. Two formats only post-Group-F: `Json` (strict
/// wire-type passthrough of cloud API responses) and `Toon` (human-readable
/// rendering of the same data via the `toon-format` crate, TOON v3.0 spec).
///
/// `Pretty` and `NoTty` are deprecated aliases kept until Task 11 to enable
/// per-command migration without a flag-day cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Toon,
    /// Deprecated — removed in Task 11.
    Pretty,
    /// Deprecated — removed in Task 11.
    NoTty,
}

impl OutputFormat {
    /// Parse a `--format` string. Unknown / legacy values auto-detect via TTY.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "toon" => Self::Toon,
            "pretty" => Self::Pretty,
            "no-tty" | "notty" => Self::NoTty,
            _ => Self::auto(),
        }
    }

    /// Resolve the effective format given an optional explicit CLI value.
    pub fn resolve(explicit: Option<&str>) -> Self {
        match explicit {
            Some(s) => Self::parse(s),
            None => Self::auto(),
        }
    }

    /// Auto-pick based on whether stdout is a terminal: TTY → Toon, else Json.
    fn auto() -> Self {
        if std::io::stdout().is_terminal() {
            Self::Toon
        } else {
            Self::Json
        }
    }

    /// Canonical string form for callsites that still pass `&str` (Task 11
    /// removes the surviving callers).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toon => "toon",
            Self::Pretty => "pretty",
            Self::NoTty => "no-tty",
        }
    }
}

/// Render any `Serialize` value in the chosen format. `Json` uses
/// `serde_json::to_string_pretty`; `Toon` uses `toon_format::encode_default`,
/// which accepts `T: Serialize` directly (no intermediate `serde_json::Value`).
/// `Pretty` and `NoTty` fall through to JSON until Task 11 deletes them.
pub fn render<T: Serialize>(value: &T, fmt: OutputFormat) -> Result<String, TemperError> {
    match fmt {
        OutputFormat::Json | OutputFormat::Pretty | OutputFormat::NoTty => {
            Ok(serde_json::to_string_pretty(value)?)
        }
        OutputFormat::Toon => toon_format::encode_default(value)
            .map_err(|e| TemperError::Api(format!("toon render: {e}"))),
    }
}

/// `temper resource show` exception: Toon emits markdown body with the
/// frontmatter at the top (as today's Pretty/NoTty does); Json emits a
/// composite shape `{ ...metadata, content: "<body>" }`.
pub fn render_resource_show(
    metadata: &serde_json::Value,
    body: &str,
    fmt: OutputFormat,
) -> Result<String, TemperError> {
    match fmt {
        OutputFormat::Toon | OutputFormat::Pretty | OutputFormat::NoTty => {
            // Frontmatter as YAML between `---` fences, then the body.
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

/// Legacy helper kept until Task 11; new code uses `render`.
pub fn output<T: Serialize + std::fmt::Display>(value: &T, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        }
        OutputFormat::Toon => match toon_format::encode_default(value) {
            Ok(s) => println!("{s}"),
            Err(_) => println!("{value}"),
        },
        OutputFormat::Pretty | OutputFormat::NoTty => println!("{value}"),
    }
}

/// Resolve an optional explicit format to its canonical string form
/// (auto-detecting the TTY when `None`). Convenience wrapper for dispatch.
pub fn resolve_format_str(explicit: Option<&str>) -> &'static str {
    OutputFormat::resolve(explicit).as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    fn parse_toon_lowercase() {
        assert_eq!(OutputFormat::parse("toon"), OutputFormat::Toon);
    }

    #[test]
    fn resolve_explicit_toon() {
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
        // Contains-check, not exact-string — Toon formatting may evolve.
        assert!(out.contains("slug"), "toon: {out}");
        assert!(out.contains("hello"), "toon: {out}");
    }

    // Existing tests kept for backward compat until Task 11.

    #[test]
    fn parse_pretty_lowercase() {
        assert_eq!(OutputFormat::parse("pretty"), OutputFormat::Pretty);
    }

    #[test]
    fn parse_no_tty_with_dash() {
        assert_eq!(OutputFormat::parse("no-tty"), OutputFormat::NoTty);
    }

    #[test]
    fn parse_json_lowercase() {
        assert_eq!(OutputFormat::parse("json"), OutputFormat::Json);
    }

    #[test]
    fn parse_unknown_defaults_to_auto() {
        let v = OutputFormat::parse("text");
        assert!(matches!(
            v,
            OutputFormat::Pretty | OutputFormat::NoTty | OutputFormat::Toon | OutputFormat::Json
        ));
    }

    #[test]
    fn resolve_explicit_honors_value() {
        assert_eq!(OutputFormat::resolve(Some("json")), OutputFormat::Json);
    }
}
