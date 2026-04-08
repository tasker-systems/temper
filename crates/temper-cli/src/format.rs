use std::io::IsTerminal;

use serde::Serialize;

/// Output format selector for CLI commands.
///
/// `Pretty` renders markdown-style pipe tables with bold headers; used when
/// stdout is a TTY and the user did not override via `--format`. `NoTty` is
/// tab-delimited with no borders and no ANSI, suited for pipes and scripts.
/// `Json` always outputs full JSON (including all frontmatter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Pretty,
    NoTty,
    Json,
}

impl OutputFormat {
    /// Parse a `--format` string. Unknown / legacy values auto-detect via TTY.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pretty" => Self::Pretty,
            "no-tty" | "notty" => Self::NoTty,
            "json" => Self::Json,
            // Legacy "text" or anything else: auto-detect
            _ => Self::auto(),
        }
    }

    /// Resolve the effective format given an optional explicit CLI value.
    ///
    /// `None` auto-detects; `Some("text")` is treated as auto-detect for
    /// backward compatibility.
    pub fn resolve(explicit: Option<&str>) -> Self {
        match explicit {
            Some(s) => Self::parse(s),
            None => Self::auto(),
        }
    }

    /// Pick a format based on whether stdout is a terminal.
    fn auto() -> Self {
        if std::io::stdout().is_terminal() {
            Self::Pretty
        } else {
            Self::NoTty
        }
    }

    /// Canonical string form for passing to existing `&str`-taking callsites.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pretty => "pretty",
            Self::NoTty => "no-tty",
            Self::Json => "json",
        }
    }
}

/// Resolve an optional explicit format to its canonical string form
/// (auto-detecting the TTY when `None`). Convenience wrapper for dispatch.
pub fn resolve_format_str(explicit: Option<&str>) -> &'static str {
    OutputFormat::resolve(explicit).as_str()
}

/// Print a serializable value in the requested format.
pub fn output<T: Serialize + std::fmt::Display>(value: &T, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            );
        }
        OutputFormat::Pretty | OutputFormat::NoTty => {
            println!("{value}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // "text" is legacy and should resolve to auto-detect (Pretty in tests
        // depends on TTY; we only check that it is one of Pretty or NoTty).
        let v = OutputFormat::parse("text");
        assert!(matches!(v, OutputFormat::Pretty | OutputFormat::NoTty));
    }

    #[test]
    fn resolve_explicit_honors_value() {
        assert_eq!(OutputFormat::resolve(Some("json")), OutputFormat::Json);
    }

    #[test]
    fn resolve_none_picks_tty_or_no_tty() {
        let v = OutputFormat::resolve(None);
        assert!(matches!(v, OutputFormat::Pretty | OutputFormat::NoTty));
    }
}
