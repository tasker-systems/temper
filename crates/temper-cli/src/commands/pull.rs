//! `temper pull` — refresh a vault file from the cloud.

use uuid::Uuid;

use crate::actions::ingest;
use crate::actions::runtime;
use crate::error::TemperError;
use crate::output;

pub fn run(resource_id: &str) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| TemperError::NotFound(format!("Invalid UUID: {e}")))?;

    runtime::with_client(|client| {
        Box::pin(async move {
            // Fetch resource metadata and content.
            let resource = client
                .resources()
                .get(id)
                .await
                .map_err(|e| TemperError::Api(e.to_string()))?;

            let content_response = client
                .resources()
                .content(id)
                .await
                .map_err(|e| TemperError::Api(e.to_string()))?;

            // Check if resource is in manifest (imported).
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());
            let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

            if let Some(entry) = manifest.entries.get_mut(&id) {
                // IMPORTED resource — write to vault path from manifest.
                let vault_path = vault_root.join(&entry.path);
                if let Some(parent) = vault_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // Parse context/doc_type from manifest path: "{context}/{doc_type}/{slug}.md"
                let parts: Vec<&str> = entry.path.split('/').collect();
                let ctx = parts.first().copied().unwrap_or("default");
                let dtype = if parts.len() > 1 {
                    parts[1]
                } else {
                    "resource"
                };

                let frontmatter =
                    ingest::build_frontmatter(id, &resource.title, ctx, dtype, None, None);
                let full_content = format!("{frontmatter}{}", content_response.markdown);
                std::fs::write(&vault_path, &full_content)?;

                // Update manifest entry.
                let content_hash = ingest::compute_content_hash(&full_content);
                entry.content_hash = content_hash;
                entry.remote_hash = resource.content_hash.unwrap_or_default();
                entry.synced_at = chrono::Utc::now();
                entry.state = temper_core::types::ManifestEntryState::Clean;
                crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

                output::success(format!(
                    "Pulled: \"{}\" -> {}",
                    resource.title,
                    vault_path.display()
                ));
            } else {
                // ADDED resource — write as snapshot to CWD.
                let filename = format!("{id}.md");
                std::fs::write(&filename, &content_response.markdown)?;
                output::success(format!("Pulled: \"{}\" -> {filename}", resource.title));
            }

            Ok(())
        })
    })
}
