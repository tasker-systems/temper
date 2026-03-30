use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(config: &Config, quiet: bool) -> Result<()> {
    let vault_ok = check_vault(config);
    let dirs_ok = check_dirs(config);
    let state_ok = check_state(config);

    if quiet {
        if let Err(ref msg) = vault_ok {
            output::error(msg);
        }
        if let Err(ref msg) = dirs_ok {
            output::error(msg);
        }
        if let Err(ref msg) = state_ok {
            output::error(format!("State: {msg}"));
        }
        return Ok(());
    }

    match &vault_ok {
        Ok(()) => output::status_icon(true, format!("Vault: {}", config.vault_root.display())),
        Err(msg) => output::status_icon(false, format!("Vault: {msg}")),
    }

    match &dirs_ok {
        Ok(()) => output::status_icon(true, "Dirs: sessions, tasks, goals, templates"),
        Err(msg) => output::warning(format!("Dirs: {msg}")),
    }

    match &state_ok {
        Ok(()) => output::status_icon(true, format!("State: {}", config.state_dir.display())),
        Err(msg) => output::status_icon(false, format!("State: {msg}")),
    }

    Ok(())
}

fn check_vault(config: &Config) -> std::result::Result<(), String> {
    if !config.vault_root.exists() {
        return Err(format!(
            "vault root does not exist: {}",
            config.vault_root.display()
        ));
    }
    let toml_path = config.vault_root.join("temper.toml");
    if !toml_path.exists() {
        return Err(format!(
            "temper.toml not found in {}",
            config.vault_root.display()
        ));
    }
    Ok(())
}

fn check_dirs(config: &Config) -> std::result::Result<(), String> {
    let dirs = [
        ("sessions", &config.sessions_dir),
        ("tasks", &config.tasks_dir),
        ("goals", &config.goals_dir),
        ("templates", &config.templates_dir),
    ];

    let missing: Vec<&str> = dirs
        .iter()
        .filter(|(_, path)| !path.exists())
        .map(|(name, _)| *name)
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing directories: {}", missing.join(", ")))
    }
}

fn check_state(config: &Config) -> std::result::Result<(), String> {
    if !config.state_dir.exists() {
        return Err(format!(
            "not initialized — run 'temper init' ({})",
            config.state_dir.display()
        ));
    }
    Ok(())
}
