//! `temper pull` — refresh a vault file from the cloud.

use uuid::Uuid;

use crate::actions::runtime;
use crate::actions::sync::pull_one_resource;
use crate::error::TemperError;
use crate::output;
use temper_core::types::ResourceId;

pub fn run(resource_id: &str) -> crate::error::Result<()> {
    let id = Uuid::parse_str(resource_id)
        .map_err(|e| TemperError::NotFound(format!("Invalid UUID: {e}")))?;
    let resource_id_typed = ResourceId::from(id);

    runtime::with_client(|client| {
        Box::pin(async move {
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());

            // Try to load a manifest; if missing, fall through to snapshot mode.
            let (mut manifest_opt, persist) =
                match crate::manifest_io::load_manifest(&temper_dir, &device_id) {
                    Ok(m) => (Some(m), true),
                    Err(_) => (None, false),
                };

            // When a manifest loads, snapshots (for untracked ids) and tracked
            // writes both go under vault_root. When there's no manifest at all,
            // snapshots must land in CWD — matching pre-refactor behavior
            // (`commands/pull.rs:88` pre-refactor: bare relative path) and the
            // design spec (Unit A acceptance criterion 3).
            let write_root: std::path::PathBuf = if manifest_opt.is_some() {
                vault_root.clone()
            } else {
                std::env::current_dir()?
            };

            let result = pull_one_resource(
                client,
                &write_root,
                resource_id_typed,
                manifest_opt.as_mut(),
            )
            .await?;

            // Fetch title for the user-facing message.
            let resource = client
                .resources()
                .get(id)
                .await
                .map_err(crate::commands::client_err)?;

            if persist {
                if let Some(m) = &manifest_opt {
                    crate::manifest_io::save_manifest(&temper_dir, m)?;
                }
            }
            output::success(format!(
                "Pulled: \"{}\" -> {}",
                resource.title,
                result.path.display()
            ));
            Ok(())
        })
    })
}
