//! Color/ANSI control for CLI output.
//!
//! Resolves an explicit color mode from `flag → TEMPER_COLOR env → config →
//! NO_COLOR → tty-default(auto)` and installs it as `anstream`'s process-global
//! color choice. Every existing `output::*` helper then obeys it with no other
//! changes — they all go through `anstream::stdout()` / `anstream::stderr()`.

/// Resolved color/ANSI choice for CLI output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Parse a color-mode string, returning `None` for unrecognized values.
///
/// Accepts case-insensitive `"auto"`, `"always"`, `"never"`; any other value
/// (including empty string) returns `None` so callers can fall through to the
/// next precedence layer.
fn parse_color_mode(s: &str) -> Option<ColorMode> {
    match s.to_lowercase().as_str() {
        "auto" => Some(ColorMode::Auto),
        "always" => Some(ColorMode::Always),
        "never" => Some(ColorMode::Never),
        _ => None,
    }
}

/// Resolve the effective color mode with precedence (highest to lowest):
/// 1. `flag`         — `--color` value (`auto`|`always`|`never`)
/// 2. `env`          — `TEMPER_COLOR` value (`auto`|`always`|`never`)
/// 3. `config`       — `[cli].color` config value (`auto`|`always`|`never`)
/// 4. `no_color_set` — the NO_COLOR convention: if set (and non-empty), force `Never`
/// 5. default        — `Auto` (anstream then does TTY detection)
///
/// At layers 1–3 an unrecognized value falls through to the next layer
/// (a typo degrades to NO_COLOR/Auto rather than locking a mode).
/// Empty strings are treated as unrecognized and also fall through.
pub fn resolve_color(
    flag: Option<&str>,
    env: Option<&str>,
    config: Option<&str>,
    no_color_set: bool,
) -> ColorMode {
    // Layer 1: explicit CLI flag
    if let Some(s) = flag {
        if let Some(mode) = parse_color_mode(s) {
            return mode;
        }
    }
    // Layer 2: TEMPER_COLOR env var (empty string treated as unset)
    if let Some(s) = env {
        if !s.is_empty() {
            if let Some(mode) = parse_color_mode(s) {
                return mode;
            }
        }
    }
    // Layer 3: config file default ([cli].color)
    if let Some(s) = config {
        if let Some(mode) = parse_color_mode(s) {
            return mode;
        }
    }
    // Layer 4: NO_COLOR convention
    if no_color_set {
        return ColorMode::Never;
    }
    // Layer 5: default — anstream handles TTY detection
    ColorMode::Auto
}

/// Read `TEMPER_COLOR` / `NO_COLOR` from the environment, combine with the
/// CLI flag and config default, and install the resulting choice as
/// anstream's process-global color choice. Call once, early in `main`,
/// before any styled output.
///
/// Resolution precedence (highest to lowest):
///   1. `flag`   — `--color` value passed in from the parsed CLI args
///   2. `TEMPER_COLOR` env var
///   3. `config` — `[cli].color` value from the loaded config
///   4. `NO_COLOR` env var (non-empty → Never)
///   5. `Auto`   — anstream performs TTY detection at stream-open time
pub fn apply_color_choice(flag: Option<&str>, config: Option<&str>) {
    let temper_color = std::env::var("TEMPER_COLOR").ok().filter(|s| !s.is_empty());
    let no_color_set = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());

    let mode = resolve_color(flag, temper_color.as_deref(), config, no_color_set);

    let choice = match mode {
        ColorMode::Auto => anstream::ColorChoice::Auto,
        ColorMode::Always => anstream::ColorChoice::Always,
        ColorMode::Never => anstream::ColorChoice::Never,
    };
    choice.write_global();
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- flag layer (1) --

    #[test]
    fn flag_never_beats_env_config_no_color() {
        assert_eq!(
            resolve_color(Some("never"), Some("always"), Some("always"), false),
            ColorMode::Never
        );
    }

    #[test]
    fn flag_always_overrides_no_color() {
        // Explicit flag wins even when NO_COLOR is set.
        assert_eq!(
            resolve_color(Some("always"), None, None, true),
            ColorMode::Always
        );
    }

    #[test]
    fn flag_auto_is_honored() {
        assert_eq!(
            resolve_color(Some("auto"), Some("never"), Some("never"), true),
            ColorMode::Auto
        );
    }

    // -- env layer (2) --

    #[test]
    fn env_beats_config_and_no_color_when_no_flag() {
        assert_eq!(
            resolve_color(None, Some("never"), Some("always"), false),
            ColorMode::Never
        );
    }

    // -- config layer (3) --

    #[test]
    fn config_used_when_no_flag_or_env() {
        assert_eq!(
            resolve_color(None, None, Some("always"), false),
            ColorMode::Always
        );
    }

    // -- NO_COLOR layer (4) --

    #[test]
    fn no_color_forces_never_when_no_flag_env_config() {
        assert_eq!(resolve_color(None, None, None, true), ColorMode::Never);
    }

    // -- default layer (5) --

    #[test]
    fn default_auto_when_nothing_set() {
        assert_eq!(resolve_color(None, None, None, false), ColorMode::Auto);
    }

    // -- fall-through / unrecognized values --

    #[test]
    fn unrecognized_flag_falls_through_to_config() {
        assert_eq!(
            resolve_color(Some("bogus"), None, Some("never"), false),
            ColorMode::Never
        );
    }

    #[test]
    fn empty_env_treated_as_unset_falls_through_to_auto() {
        assert_eq!(resolve_color(None, Some(""), None, false), ColorMode::Auto);
    }

    #[test]
    fn unrecognized_env_falls_through_to_config() {
        assert_eq!(
            resolve_color(None, Some("garbage"), Some("always"), false),
            ColorMode::Always
        );
    }

    #[test]
    fn unrecognized_config_falls_through_to_no_color() {
        assert_eq!(
            resolve_color(None, None, Some("garbage"), true),
            ColorMode::Never
        );
    }

    #[test]
    fn unrecognized_config_falls_through_to_auto_default() {
        assert_eq!(
            resolve_color(None, None, Some("garbage"), false),
            ColorMode::Auto
        );
    }

    // -- case-insensitivity --

    #[test]
    fn flag_always_uppercase_is_recognized() {
        assert_eq!(
            resolve_color(Some("ALWAYS"), None, None, false),
            ColorMode::Always
        );
    }

    #[test]
    fn flag_never_mixed_case_is_recognized() {
        assert_eq!(
            resolve_color(Some("Never"), None, None, false),
            ColorMode::Never
        );
    }

    #[test]
    fn env_auto_uppercase_is_recognized() {
        assert_eq!(
            resolve_color(None, Some("AUTO"), None, false),
            ColorMode::Auto
        );
    }

    // -- smoke test: apply_color_choice does not panic --

    #[test]
    fn apply_color_choice_does_not_panic() {
        temp_env::with_vars([("TEMPER_COLOR", Some("auto")), ("NO_COLOR", None)], || {
            apply_color_choice(None, None);
        });
    }
}
