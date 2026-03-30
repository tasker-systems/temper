use std::path::Path;

use crate::config::{global_config_path, GlobalConfig};
use crate::error::Result;
use crate::output;

const TEMPER_TOML: &str = r#"[vault]
# sessions = "sessions"
# tasks = "tasks"
# goals = "goals"
# templates = "templates"
# state_dir = ".temper"

[skill]
# output = "~/.claude/commands/temper.md"
# framework = "superpowers"
"#;

const EMBEDDED_SESSION: &str = include_str!("../templates/session.md");
const EMBEDDED_TASK: &str = include_str!("../templates/task.md");
const EMBEDDED_GOAL: &str = include_str!("../templates/goal.md");

/// Run temper init.
///
/// - `no_interactive`: skip interactive prompts
/// - `register_global`: write default_vault to `~/.config/temper/config.toml`.
///   Pass `false` from tests to avoid clobbering the user's real global config.
pub fn run(path: &Path, no_interactive: bool, register_global: bool) -> Result<()> {
    // 1. Create vault directory
    output::dim(format!("Creating vault at {}", path.display()));
    std::fs::create_dir_all(path)?;

    // 2. Write temper.toml
    let toml_path = path.join("temper.toml");
    if toml_path.exists() {
        output::dim("temper.toml already exists, skipping");
    } else {
        std::fs::write(&toml_path, TEMPER_TOML)?;
        output::success("Wrote temper.toml");
    }

    // 3. Create essential directories
    for dir in &["sessions", "tasks", "goals", "templates"] {
        let dir_path = path.join(dir);
        std::fs::create_dir_all(&dir_path)?;
        output::item(format!("Created {dir}/"));
    }

    // 4. Write embedded templates
    let templates_dir = path.join("templates");
    write_template_if_missing(&templates_dir.join("session.md"), EMBEDDED_SESSION)?;

    write_template_if_missing(&templates_dir.join("task.md"), EMBEDDED_TASK)?;
    write_template_if_missing(&templates_dir.join("goal.md"), EMBEDDED_GOAL)?;

    // 5. Register as default vault in ~/.config/temper/config.toml if none exists
    if register_global {
        register_default_vault(path)?;
    }

    // 6. Interactive guidance
    if !no_interactive {
        output::blank();
        output::success("Vault initialized successfully");
        output::blank();
        output::header("Next steps");
        output::hint("  temper check          — verify vault and tool health");
        output::hint("  temper note create session \"My First Session\"");
        output::hint("  temper task create --title \"First Task\" --context myproject");
        output::blank();
        output::hint("To generate a Claude skill for this vault:");
        output::hint("  temper skill generate  (coming soon)");
    }

    Ok(())
}

fn write_template_if_missing(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, content)?;
    output::item(format!(
        "Wrote {}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    Ok(())
}

fn register_default_vault(vault_path: &Path) -> Result<()> {
    let config_path = global_config_path();

    // Check if the global config already has a default vault set
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).unwrap_or_default();
        let existing: GlobalConfig = toml::from_str(&content).unwrap_or_default();
        if existing.default_vault.is_some() {
            output::dim("Global config already has default_vault set, skipping");
            return Ok(());
        }
    }

    // Create parent dirs if needed
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let canonical = vault_path
        .canonicalize()
        .unwrap_or_else(|_| vault_path.to_path_buf());

    let new_config = GlobalConfig {
        default_vault: Some(canonical.to_string_lossy().to_string()),
    };

    let toml_content = toml::to_string(&new_config).map_err(|e| {
        crate::error::TemperError::Config(format!("failed to serialize global config: {e}"))
    })?;

    std::fs::write(&config_path, toml_content)?;
    output::dim(format!(
        "Registered {} as default vault in {}",
        canonical.display(),
        config_path.display()
    ));

    Ok(())
}
