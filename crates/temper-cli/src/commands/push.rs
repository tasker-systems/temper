//! `temper push` — upload a single resource to the cloud.

use uuid::Uuid;

use crate::actions::runtime;
use crate::actions::sync::{push_one_resource, PushTarget};
use crate::error::TemperError;
use crate::output;
use temper_core::types::ResourceId;

/// Accept either a UUID (requires manifest) or a filesystem path (absolute,
/// CWD-relative, or vault-root-relative).
pub fn run(target: &str) -> crate::error::Result<()> {
    let target_owned = target.to_string();

    runtime::with_client(|client| {
        Box::pin(async move {
            let vault_root = crate::config::resolve_vault(None)?;
            let temper_dir = vault_root.join(".temper");
            let device_id =
                crate::config::load_device_id().unwrap_or_else(|| "unknown".to_string());

            // Try to load a manifest; if absent, proceed manifest-less.
            // (Manifest-less push is the cloud-mode-B.2 path — works today for
            //  users running `temper push` in a working directory without a
            //  vault.)
            let (mut manifest_opt, persist) =
                match crate::manifest_io::load_manifest(&temper_dir, &device_id) {
                    Ok(m) => (Some(m), true),
                    Err(_) => (None, false),
                };

            // UUID first — if it parses, treat as id target (requires manifest).
            // Else resolve as a path: CWD-relative, then vault-root-relative.
            let result = if let Ok(uuid) = Uuid::parse_str(&target_owned) {
                push_one_resource(
                    client,
                    &vault_root,
                    PushTarget::Id(ResourceId::from(uuid)),
                    manifest_opt.as_mut(),
                )
                .await?
            } else {
                let cwd_path = std::env::current_dir()?.join(&target_owned);
                let resolved: std::path::PathBuf = if cwd_path.exists() {
                    cwd_path
                } else {
                    let vr = vault_root.join(&target_owned);
                    if !vr.exists() {
                        return Err(TemperError::NotFound(format!(
                            "push target not found: {target_owned}"
                        )));
                    }
                    vr
                };
                push_one_resource(
                    client,
                    &vault_root,
                    PushTarget::Path(&resolved),
                    manifest_opt.as_mut(),
                )
                .await?
            };

            // Persist manifest BEFORE printing (commit-then-report pattern —
            // same as commands/pull.rs after Task 2's fix).
            if persist {
                if let Some(m) = &manifest_opt {
                    crate::manifest_io::save_manifest(&temper_dir, m)?;
                }
            }

            output::success(format!(
                "Pushed: {} -> {}",
                result.path.display(),
                result.resource_id
            ));
            Ok(())
        })
    })
}
