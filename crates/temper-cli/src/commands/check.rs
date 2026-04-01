use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(config: &Config, quiet: bool) -> Result<()> {
    let vault_ok = check_vault(config);
    let state_ok = check_state(config);

    if quiet {
        if let Err(ref msg) = vault_ok {
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

    match &state_ok {
        Ok(()) => output::status_icon(true, format!("State: {}", config.state_dir.display())),
        Err(msg) => output::status_icon(false, format!("State: {msg}")),
    }

    // Show contexts
    if !config.contexts.is_empty() {
        output::status_icon(true, format!("Contexts: {}", config.contexts.join(", ")));
    } else {
        output::warning("Contexts: none configured");
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
    Ok(())
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
