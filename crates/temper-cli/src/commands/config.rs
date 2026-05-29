//! `temper config edit` command entry point.

use std::path::Path;

use dialoguer::{theme::ColorfulTheme, Select};

use crate::actions::config as action;
use crate::error::{Result, TemperError};
use crate::output;
use temper_core::types::config::{global_config_path, TemperConfig};

/// Open `$EDITOR` against a temp copy of the global config and loop until
/// the edited TOML is both structurally and semantically valid, or the user
/// chooses to discard.
pub fn edit() -> Result<()> {
    let target = global_config_path();
    ensure_config_exists(&target)?;

    let edit_path = action::temp_edit_path(&target);

    // A stale .edit file means a previous invocation was interrupted
    // (Ctrl+C, editor crash, etc). Warn and start fresh rather than
    // silently overwriting — the user should know their prior edit is
    // being discarded.
    if edit_path.exists() {
        output::warning(format!(
            "discarding stale edit file from a previous interrupted session: {}",
            edit_path.display()
        ));
        std::fs::remove_file(&edit_path)
            .map_err(|e| TemperError::Config(format!("cannot remove stale edit file: {e}")))?;
    }

    std::fs::copy(&target, &edit_path)
        .map_err(|e| TemperError::Config(format!("cannot copy for edit: {e}")))?;

    loop {
        open_in_editor(&edit_path)?;
        let content = std::fs::read_to_string(&edit_path)
            .map_err(|e| TemperError::Config(format!("cannot read edit file: {e}")))?;

        match action::parse_and_validate(&content) {
            action::ParseOutcome::Valid(_) => {
                action::commit_edit(&edit_path, &target)?;
                output::success(format!("Config saved: {}", target.display()));
                return Ok(());
            }
            action::ParseOutcome::Invalid(msg) => {
                output::error(msg);
                let choices = ["Re-edit", "Discard changes"];
                let idx = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("What now?")
                    .default(0)
                    .items(choices)
                    .interact()
                    .map_err(|e| TemperError::Config(format!("prompt error: {e}")))?;
                if idx == 1 {
                    let _ = std::fs::remove_file(&edit_path);
                    output::warning("Discarded changes");
                    return Ok(());
                }
            }
        }
    }
}

fn ensure_config_exists(target: &Path) -> Result<()> {
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TemperError::Config(format!("cannot create config dir: {e}")))?;
    }
    let default_config = TemperConfig::default();
    let toml = toml::to_string_pretty(&default_config)
        .map_err(|e| TemperError::Config(format!("default config serialize: {e}")))?;
    std::fs::write(target, toml)
        .map_err(|e| TemperError::Config(format!("cannot write default config: {e}")))?;
    output::dim(format!("Seeded default config at {}", target.display()));
    Ok(())
}

fn open_in_editor(path: &Path) -> Result<()> {
    // Respect $VISUAL before $EDITOR (POSIX convention — $VISUAL is the
    // interactive editor, $EDITOR is the fallback for dumb terminals).
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .map_err(|_| {
            TemperError::Config(
                "Set $VISUAL or $EDITOR to use `temper config edit`, \
                 e.g. export EDITOR=vim"
                    .into(),
            )
        })?;

    // Split on shell-quoting rules so editors configured with arguments
    // work: EDITOR="code --wait", EDITOR="emacsclient -nw", etc.
    let parts = shell_words::split(&editor)
        .map_err(|e| TemperError::Config(format!("cannot parse $EDITOR {editor:?}: {e}")))?;
    let (program, args) = parts
        .split_first()
        .ok_or_else(|| TemperError::Config("$EDITOR is empty".into()))?;

    let status = std::process::Command::new(program)
        .args(args)
        .arg(path)
        .status()
        .map_err(|e| TemperError::Config(format!("failed to launch {program}: {e}")))?;
    if !status.success() {
        return Err(TemperError::Config(format!(
            "{program} exited with status {status}"
        )));
    }
    Ok(())
}
