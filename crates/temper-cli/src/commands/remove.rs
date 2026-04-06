//! `temper remove` — delete a resource from the cloud and optionally the vault.

use temper_core::types::ResourceId;
use uuid::Uuid;

use crate::actions::runtime;
use crate::error::TemperError;
use crate::output;

pub fn run(resource_id: &str, force: bool) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| TemperError::NotFound(format!("Invalid UUID: {e}")))?;
    let rid = ResourceId::from(id);

    runtime::with_client(|client| {
        Box::pin(async move {
            // Delete from cloud.
            client
                .resources()
                .delete(id)
                .await
                .map_err(|e| TemperError::Api(e.to_string()))?;
            output::success(format!("Deleted from cloud: {id}"));

            // Check if in manifest.
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());
            let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

            if let Some(entry) = manifest.entries.get(&rid) {
                let vault_path = vault_root.join(&entry.path);

                let should_remove = if force {
                    true
                } else {
                    output::progress(format!(
                        "Also remove vault file at {}? [y/N] ",
                        vault_path.display()
                    ));
                    use std::io::Write as _;
                    std::io::stderr().flush().ok();
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input).ok();
                    input.trim().eq_ignore_ascii_case("y")
                };

                if should_remove {
                    if vault_path.exists() {
                        std::fs::remove_file(&vault_path)?;
                        output::dim(format!("Removed vault file: {}", vault_path.display()));
                    }
                    manifest.entries.remove(&rid);
                    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;
                }
            }

            Ok(())
        })
    })
}
