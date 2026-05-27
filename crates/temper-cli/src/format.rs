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
        match s.to_lowercase().as_str() {
            "json" => Self::Json,
            "toon" => Self::Toon,
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

/// Resolve an optional explicit format to its canonical string form
/// (auto-detecting the TTY when `None`). Convenience wrapper for callsites
/// that still pass `&str` (Warmup, search) — these can migrate to `resolve`
/// directly in a future cleanup.
pub fn resolve_format_str(explicit: Option<&str>) -> &'static str {
    OutputFormat::resolve(explicit).as_str()
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
}
