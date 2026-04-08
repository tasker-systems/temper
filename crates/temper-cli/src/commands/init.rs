//! `temper init` — guided vault + config setup.
//!
//! The wizard is split into two parts so that tests can drive the apply
//! step without touching dialoguer: `gather_answers` (interactive) and
//! `apply_answers` (pure disk work).

use std::path::{Path, PathBuf};

use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};

use crate::config::global_config_path;
use crate::error::{Result, TemperError};
use crate::output;

/// User selection for auth provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthChoice {
    Auth0,
    None,
}

/// Collected wizard answers — produced by `gather_answers` (interactive) or
/// `default_answers` (`--no-interactive`).
#[derive(Debug, Clone)]
pub struct WizardAnswers {
    pub vault_path: String,
    pub extra_contexts: Vec<String>,
    pub auth_choice: AuthChoice,
}

fn default_vault_path() -> String {
    dirs::home_dir()
        .map(|h| {
            h.join("Documents/temper-vault")
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "./temper-vault".to_string())
}

/// Resolve an initial vault path from a CLI argument: an empty argument
/// falls back to `default_vault_path()`. Shared between interactive and
/// non-interactive entry points so both handle the empty case identically.
fn resolve_initial_vault(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        default_vault_path()
    } else {
        path.to_string_lossy().to_string()
    }
}

/// Convert a dialoguer prompt failure into a `TemperError`. Prompt errors
/// are a configuration-setup problem, not a vault-state problem, so we map
/// to `Config` rather than `Vault`.
fn prompt_err(e: dialoguer::Error) -> TemperError {
    TemperError::Config(format!("prompt error: {e}"))
}

/// CLI entry point dispatched from `main.rs`.
pub fn run(path: &Path, no_interactive: bool, register_global: bool) -> Result<()> {
    if no_interactive {
        return run_non_interactive(path, register_global);
    }
    let initial_vault = resolve_initial_vault(path);
    let answers = gather_answers(&initial_vault)?;
    print_summary(&answers, register_global);
    let proceed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Proceed?")
        .default(true)
        .interact()
        .map_err(prompt_err)?;
    if !proceed {
        output::warning("Init cancelled");
        return Ok(());
    }
    apply_answers(&answers, register_global)
}

/// Non-interactive path — uses all defaults.
pub fn run_non_interactive(path: &Path, register_global: bool) -> Result<()> {
    let answers = WizardAnswers {
        vault_path: resolve_initial_vault(path),
        extra_contexts: Vec::new(),
        auth_choice: AuthChoice::Auth0,
    };
    apply_answers(&answers, register_global)
}

/// Run the interactive prompts and return collected answers.
fn gather_answers(initial_vault: &str) -> Result<WizardAnswers> {
    let theme = ColorfulTheme::default();

    let vault_path: String = Input::with_theme(&theme)
        .with_prompt("Where should your vault live?")
        .default(initial_vault.to_string())
        .interact_text()
        .map_err(prompt_err)?;

    let contexts_raw: String = Input::with_theme(&theme)
        .with_prompt("Create any contexts now? (comma-separated, or Enter for just 'default')")
        .default(String::new())
        .allow_empty(true)
        .interact_text()
        .map_err(prompt_err)?;

    let extra_contexts: Vec<String> = contexts_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "default")
        .collect();

    let items = [
        "auth0 (recommended — temperkb.io cloud sync)",
        "none (local-only, no sync)",
    ];
    let idx = Select::with_theme(&theme)
        .with_prompt("Auth provider")
        .default(0)
        .items(&items)
        .interact()
        .map_err(prompt_err)?;

    let auth_choice = if idx == 0 {
        AuthChoice::Auth0
    } else {
        AuthChoice::None
    };

    Ok(WizardAnswers {
        vault_path,
        extra_contexts,
        auth_choice,
    })
}

fn print_summary(answers: &WizardAnswers, register_global: bool) {
    output::blank();
    output::header("Ready to initialize:");
    output::label("Vault", &answers.vault_path);
    let mut ctxs = vec!["default".to_string()];
    ctxs.extend(answers.extra_contexts.iter().cloned());
    output::label("Contexts", ctxs.join(", "));
    let auth_label = match answers.auth_choice {
        AuthChoice::Auth0 => "auth0",
        AuthChoice::None => "none",
    };
    output::label("Auth", auth_label);
    if register_global {
        output::label("Config", global_config_path().display().to_string());
    }
    output::blank();
}

/// Write vault dirs and (optionally) the global config file.
pub fn apply_answers(answers: &WizardAnswers, register_global: bool) -> Result<()> {
    let vault = PathBuf::from(&answers.vault_path);

    // Warn if a .temper/ marker already exists — per Decision 1 we do not
    // offer to reconfigure, we just point the user at `temper config edit`.
    let marker = vault.join(".temper");
    if marker.exists() {
        output::warning(format!(
            "vault already exists at {}; re-running init is idempotent. \
             To change settings, run `temper config edit`.",
            vault.display()
        ));
    }

    std::fs::create_dir_all(&vault)?;

    let state_dir = vault.join(".temper");
    std::fs::create_dir_all(&state_dir)?;
    let manifest_path = state_dir.join("manifest.json");
    if !manifest_path.exists() {
        std::fs::write(&manifest_path, "{}\n")?;
    }
    let events_path = state_dir.join("events.jsonl");
    if !events_path.exists() {
        std::fs::write(&events_path, "")?;
    }

    // Create default/ and any extra contexts
    std::fs::create_dir_all(vault.join("default"))?;
    for ctx in &answers.extra_contexts {
        std::fs::create_dir_all(vault.join(ctx))?;
    }

    if register_global {
        let config_path = global_config_path();
        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let toml = render_config_toml(answers);
            std::fs::write(&config_path, toml)?;
            output::dim(format!("Wrote global config to {}", config_path.display()));
        } else {
            output::dim("Global config already exists, skipping");
        }
    }

    output::success("Vault initialized successfully");
    Ok(())
}

/// Produce the TOML body for `config.toml` from the collected answers.
///
/// Both the vault path and each context name are routed through
/// `toml::Value::String` so that characters requiring escaping (backslashes,
/// double quotes, control characters) round-trip through `TemperConfig`
/// parsing — including Windows-style paths.
pub fn render_config_toml(answers: &WizardAnswers) -> String {
    let mut ctxs = vec!["default".to_string()];
    ctxs.extend(answers.extra_contexts.iter().cloned());
    let ctx_list = ctxs
        .iter()
        .map(|c| toml::Value::String(c.clone()).to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let auth_section = match answers.auth_choice {
        AuthChoice::None => "[auth]\nprovider = \"none\"\n".to_string(),
        AuthChoice::Auth0 => r#"[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#
        .to_string(),
    };

    // toml::Value::String already includes the surrounding quotes, so the
    // `path =` line below does NOT wrap `{vault_path_toml}` in its own quotes.
    let vault_path_toml = toml::Value::String(answers.vault_path.clone()).to_string();

    format!(
        r#"[vault]
path = {vault_path_toml}

[sync.subscriptions]
contexts = [{ctx_list}]

[skill]
output = "~/.claude/skills/temper"

{auth_section}
[cloud]
api_url = "https://temperkb.io"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::config::TemperConfig;
    use validator::Validate;

    /// Round-trip guard: the rendered TOML must parse into a valid
    /// `TemperConfig` for every auth choice. This catches template drift
    /// that string-contains assertions would miss.
    #[test]
    fn rendered_toml_parses_and_validates_auth0() {
        let answers = WizardAnswers {
            vault_path: "/tmp/roundtrip".into(),
            extra_contexts: vec!["one".into(), "two".into()],
            auth_choice: AuthChoice::Auth0,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig =
            toml::from_str(&rendered).expect("rendered TOML should parse into TemperConfig");
        cfg.validate().expect("rendered config should validate");
    }

    #[test]
    fn rendered_toml_parses_and_validates_auth_none() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig =
            toml::from_str(&rendered).expect("rendered TOML should parse into TemperConfig");
        cfg.validate().expect("rendered config should validate");
    }

    #[test]
    fn rendered_toml_escapes_backslashes_in_vault_path() {
        // Windows-style path — backslashes MUST survive the render/parse
        // round-trip without breaking the TOML or being dropped.
        let answers = WizardAnswers {
            vault_path: r"C:\Users\alice\Documents\temper-vault".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&rendered).expect("escaped path must parse");
        assert_eq!(cfg.vault.path, r"C:\Users\alice\Documents\temper-vault");
    }

    #[test]
    fn rendered_toml_escapes_double_quotes_in_vault_path() {
        // Pathological but valid on Unix: a path containing a double quote
        // must not break the TOML basic string it's interpolated into.
        let answers = WizardAnswers {
            vault_path: r#"/tmp/weird"name"#.into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let rendered = render_config_toml(&answers);
        let cfg: TemperConfig = toml::from_str(&rendered).expect("escaped quote must parse");
        assert_eq!(cfg.vault.path, r#"/tmp/weird"name"#);
    }

    #[test]
    fn default_answers_generate_complete_config() {
        let answers = WizardAnswers {
            vault_path: "/tmp/my-vault".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Auth0,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"path = "/tmp/my-vault""#));
        assert!(toml.contains("[auth]"));
        assert!(toml.contains(r#"provider = "auth0""#));
        assert!(toml.contains("[[auth.providers]]"));
        assert!(toml.contains(r#"name = "auth0""#));
        assert!(toml.contains("[cloud]"));
        assert!(toml.contains(r#"api_url = "https://temperkb.io""#));
        // Must NOT contain removed fields
        assert!(!toml.contains("[cli]"), "cli section should not be written");
        assert!(
            !toml.contains("framework ="),
            "skill.framework should not be written"
        );
    }

    #[test]
    fn auth_none_writes_provider_none_marker() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::None,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"provider = "none""#));
        // no auth0 provider entry when none chosen
        assert!(!toml.contains("[[auth.providers]]"));
    }

    #[test]
    fn auth0_writes_array_of_tables_format() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Auth0,
        };
        let toml = render_config_toml(&answers);
        // Must use the new array-of-tables format, NOT the old dotted-map form
        assert!(toml.contains("[[auth.providers]]"));
        assert!(toml.contains(r#"name = "auth0""#));
        assert!(
            !toml.contains("[auth.providers.auth0]"),
            "must not use old dotted form"
        );
    }

    #[test]
    fn apply_answers_warns_on_existing_vault_but_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("existing");
        // Pre-create a .temper/ marker to simulate an existing vault
        std::fs::create_dir_all(vault.join(".temper")).unwrap();
        let answers = WizardAnswers {
            vault_path: vault.to_string_lossy().to_string(),
            extra_contexts: vec![],
            auth_choice: AuthChoice::Auth0,
        };
        // Should succeed (no error) — the existing-vault warning is emitted via output::warning
        // and does not block. Test verifies idempotent behavior.
        apply_answers(&answers, false).expect("should warn but succeed");
        assert!(vault.join(".temper/manifest.json").exists());
    }

    #[test]
    fn extra_contexts_go_into_subscriptions() {
        let answers = WizardAnswers {
            vault_path: "/tmp/v".into(),
            extra_contexts: vec!["temper".into(), "writing".into()],
            auth_choice: AuthChoice::Auth0,
        };
        let toml = render_config_toml(&answers);
        assert!(toml.contains(r#"contexts = ["default", "temper", "writing"]"#));
    }

    #[test]
    fn apply_answers_creates_vault_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_path = tmp.path().join("vault");
        let answers = WizardAnswers {
            vault_path: vault_path.to_string_lossy().to_string(),
            extra_contexts: vec!["writing".into()],
            auth_choice: AuthChoice::None,
        };
        apply_answers(&answers, false).expect("apply should succeed");
        assert!(vault_path.join(".temper/manifest.json").exists());
        assert!(vault_path.join(".temper/events.jsonl").exists());
        assert!(vault_path.join("default").exists());
        assert!(vault_path.join("writing").exists());
    }

    #[test]
    fn no_interactive_defaults_and_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("v");
        run_non_interactive(&vault, false).expect("non-interactive run should succeed");
        assert!(vault.join(".temper").exists());
        assert!(vault.join("default").exists());
    }
}
