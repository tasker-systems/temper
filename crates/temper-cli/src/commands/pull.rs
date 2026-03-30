//! `temper pull` — refresh a vault file from the cloud.

use uuid::Uuid;

use crate::error::TemperError;

pub fn run(resource_id: &str) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| TemperError::Config(format!("Invalid UUID: {e}")))?;

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Config(format!("tokio runtime: {e}")))?;

    rt.block_on(async {
        let client = temper_client::config::build_client()
            .map_err(|e| TemperError::Config(e.to_string()))?;

        // Fetch resource metadata and content.
        let resource = client
            .resources()
            .get(id)
            .await
            .map_err(|e| TemperError::Config(e.to_string()))?;

        let content_response = client
            .resources()
            .content(id)
            .await
            .map_err(|e| TemperError::Config(e.to_string()))?;

        // Check if resource is in manifest (imported).
        let vault_root = crate::config::resolve_vault(None)?;
        let temper_dir = vault_root.join(".temper");
        let device_id = load_device_id().unwrap_or_else(|| "unknown".to_string());
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

        if let Some(entry) = manifest.entries.get_mut(&id) {
            // IMPORTED resource — write to vault path from manifest.
            let vault_path = vault_root.join(&entry.path);
            if let Some(parent) = vault_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Parse context/doc_type from manifest path: "{context}/{doc_type}/{uuid}.md"
            let parts: Vec<&str> = entry.path.split('/').collect();
            let ctx = parts.first().copied().unwrap_or("default");
            let dtype = if parts.len() > 1 {
                parts[1]
            } else {
                "resource"
            };

            let frontmatter = crate::commands::import_cmd::build_frontmatter(
                id,
                &resource.title,
                ctx,
                dtype,
                None,
            );
            let full_content = format!("{frontmatter}{}", content_response.markdown);
            std::fs::write(&vault_path, &full_content)?;

            // Update manifest entry.
            let content_hash = crate::commands::add::compute_content_hash(&full_content);
            entry.content_hash = content_hash;
            entry.remote_hash = resource.content_hash.unwrap_or_default();
            entry.synced_at = chrono::Utc::now();
            entry.state = temper_core::types::ManifestEntryState::Clean;
            crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

            println!(
                "\u{2713} Pulled: \"{}\" \u{2192} {}",
                resource.title,
                vault_path.display()
            );
        } else {
            // ADDED resource — write as snapshot to CWD.
            let filename = format!("{id}.md");
            std::fs::write(&filename, &content_response.markdown)?;
            println!(
                "\u{2713} Pulled: \"{}\" \u{2192} {filename}",
                resource.title
            );
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
