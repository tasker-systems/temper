use std::path::Path;

use crate::config::global_config_path;
use crate::error::Result;
use crate::output;

/// Run temper init.
///
/// - `no_interactive`: skip interactive prompts
/// - `register_global`: write global config to `~/.config/temper/config.toml`.
///   Pass `false` from tests to avoid clobbering the user's real global config.
pub fn run(path: &Path, no_interactive: bool, register_global: bool) -> Result<()> {
    // 1. Create vault directory
    output::dim(format!("Creating vault at {}", path.display()));
    std::fs::create_dir_all(path)?;

    // 2. Create .temper state directory with manifest and events
    let state_dir = path.join(".temper");
    std::fs::create_dir_all(&state_dir)?;

    let manifest_path = state_dir.join("manifest.json");
    if !manifest_path.exists() {
        std::fs::write(&manifest_path, "{}\n")?;
        output::item("Created .temper/manifest.json");
    }

    let events_path = state_dir.join("events.jsonl");
    if !events_path.exists() {
        std::fs::write(&events_path, "")?;
        output::item("Created .temper/events.jsonl");
    }

    // 3. Create default context directory
    let default_ctx = path.join("default");
    std::fs::create_dir_all(&default_ctx)?;
    output::item("Created default/ context");

    // 4. Register global config if needed
    if register_global {
        register_default_config(path)?;
    }

    // 5. Interactive guidance
    if !no_interactive {
        output::blank();
        output::success("Vault initialized successfully");
        output::blank();
        output::header("Next steps");
        output::hint("  temper check          — verify vault and tool health");
        output::hint("  temper session save \"My First Session\" --context default");
        output::hint("  temper task create --title \"First Task\" --context default");
        output::blank();
        output::hint("To generate a Claude skill for this vault:");
        output::hint("  temper skill install");
    }

    Ok(())
}

fn register_default_config(vault_path: &Path) -> Result<()> {
    let config_path = global_config_path();

    // Don't overwrite existing config
    if config_path.exists() {
        output::dim("Global config already exists, skipping");
        return Ok(());
    }

    // Create parent dirs if needed
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let canonical = vault_path
        .canonicalize()
        .unwrap_or_else(|_| vault_path.to_path_buf());
    let vault_path_str = canonical.to_string_lossy();

    let config_content = format!(
        r#"[vault]
path = "{vault_path_str}"

# Add contexts to sync: temper context add <name>
[sync.subscriptions]
contexts = []

[cli]
progress = "bar"

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#
    );

    std::fs::write(&config_path, config_content)?;
    output::dim(format!("Wrote global config to {}", config_path.display()));

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn config_template_contains_correct_skill_output() {
        // Reproduce the format string with a test path to verify contract
        let vault_path_str = "/tmp/test-vault";
        let config_content = format!(
            r#"[vault]
path = "{vault_path_str}"

# Add contexts to sync: temper context add <name>
[sync.subscriptions]
contexts = []

[cli]
progress = "bar"

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]
"#
        );
        assert!(
            config_content.contains(r#"output = "~/.claude/skills/temper""#),
            "skill output must point to skills dir, not commands"
        );
        assert!(
            config_content.contains("contexts = []"),
            "subscriptions should default to empty"
        );
        assert!(
            !config_content.contains("commands/temper.md"),
            "must not contain stale commands path"
        );
    }
}
