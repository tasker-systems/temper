use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(config: &Config, quiet: bool) -> Result<()> {
    let vault_ok = check_vault(config);
    let dirs_ok = check_dirs(config);
    let model_status = check_embedding_model(config);
    let state_ok = check_state(config);

    if quiet {
        if let Err(ref msg) = vault_ok {
            output::error(msg);
        }
        if let Err(ref msg) = dirs_ok {
            output::error(msg);
        }
        if let Err(ref msg) = model_status {
            output::error(format!("Embedding: {msg}"));
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
        Ok(()) => output::status_icon(true, "Dirs: sessions, tickets, milestones, templates"),
        Err(msg) => output::warning(format!("Dirs: {msg}")),
    }

    match &model_status {
        Ok(size_mb) => {
            output::status_icon(true, format!("Embedding: model cached, {size_mb:.1}MB"))
        }
        Err(msg) => output::status_icon(false, format!("Embedding: {msg}")),
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
        ("tickets", &config.tickets_dir),
        ("milestones", &config.milestones_dir),
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

fn check_embedding_model(config: &Config) -> std::result::Result<f64, String> {
    // Check the configured model cache dir first
    let model_dir = &config.model_cache_dir;
    if model_dir.exists() {
        let size = dir_size(model_dir).unwrap_or(0) as f64 / (1024.0 * 1024.0);
        if size > 0.0 {
            return Ok(size);
        }
    }

    // Fall back to HuggingFace hub cache
    let hf_cache = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
        .join("huggingface/hub/models--sentence-transformers--all-MiniLM-L6-v2");

    if hf_cache.exists() {
        let size = dir_size(&hf_cache).unwrap_or(0) as f64 / (1024.0 * 1024.0);
        Ok(size)
    } else {
        Err("not downloaded — run 'temper index embed' to fetch".into())
    }
}

fn check_state(config: &Config) -> std::result::Result<(), String> {
    if !config.state_dir.exists() {
        return Err(format!(
            "not initialized — run 'temper index embed' ({})",
            config.state_dir.display()
        ));
    }
    Ok(())
}

fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut size = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_file() {
                size += meta.len();
            } else if meta.is_dir() {
                size += dir_size(&entry.path())?;
            }
        }
    }
    Ok(size)
}
