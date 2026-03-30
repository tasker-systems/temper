//! `temper remove` — delete a resource from the cloud and optionally the vault.

use uuid::Uuid;

use crate::error::TemperError;

pub fn run(resource_id: &str, force: bool) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| TemperError::Config(format!("Invalid UUID: {e}")))?;

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| TemperError::Config(e.to_string()))?;

        // Delete from cloud.
        client
            .resources()
            .delete(id)
            .await
            .map_err(|e| TemperError::Config(e.to_string()))?;
        println!("\u{2713} Deleted from cloud: {id}");

        // Check if in manifest.
        let vault_root = crate::config::resolve_vault(None)?;
        let temper_dir = vault_root.join(".temper");
        let device_id = load_device_id().unwrap_or_else(|| "unknown".to_string());
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

        if let Some(entry) = manifest.entries.get(&id) {
            let vault_path = vault_root.join(&entry.path);

            let should_remove = if force {
                true
            } else {
                eprint!("Also remove vault file at {}? [y/N] ", vault_path.display());
                use std::io::Write as _;
                std::io::stderr().flush().ok();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                input.trim().eq_ignore_ascii_case("y")
            };

            if should_remove {
                if vault_path.exists() {
                    std::fs::remove_file(&vault_path)?;
                    println!("  Removed vault file: {}", vault_path.display());
                }
                manifest.entries.remove(&id);
                crate::manifest_io::save_manifest(&temper_dir, &manifest)?;
            }
        }

        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the device UUID string from `~/.config/temper/device.json`.
fn load_device_id() -> Option<String> {
    let path = dirs::home_dir()?
        .join(".config")
        .join("temper")
        .join("device.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("client_id")?.as_str().map(String::from)
}
