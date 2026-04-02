//! `temper sync` — bidirectional sync between local vault and temper cloud.
//!
//! Two subcommands:
//! - `temper sync run` — full sync cycle (push, pull, remove, complete)
//! - `temper sync status` — dry-run diff (no changes)

use crate::actions::{runtime, sync as sync_actions};
use crate::error::Result;
use crate::format::OutputFormat;
use crate::output;

/// Run a full sync cycle.
pub fn run(contexts: &[String], format: &str) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;

    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let (rt, client) = runtime::build_runtime_and_client()?;

    // Ensure profile exists before hitting sync endpoints
    rt.block_on(runtime::ensure_profile(&client))?;

    let result = rt.block_on(async {
        sync_actions::sync_orchestration(&client, &mut manifest, &vault_root, contexts).await
    })?;

    // Save manifest after successful sync
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "event": "sync_complete",
            "pushed": result.push_count,
            "pulled": result.pull_count,
            "conflicts": result.conflict_count,
            "removed": result.removed_count,
        });
        output::plain(event);
    } else {
        output::plain(format!(
            "  \u{2191} Push    {} resources",
            result.push_count
        ));
        output::plain(format!(
            "  \u{2193} Pull    {} resources",
            result.pull_count
        ));
        if result.conflict_count > 0 {
            output::plain(format!(
                "  \u{2717} Conflict {} resources (skipped)",
                result.conflict_count
            ));
        }
        if result.removed_count > 0 {
            output::plain(format!(
                "  \u{2212} Removed {} resources",
                result.removed_count
            ));
        }
        let total = result.push_count + result.pull_count + result.removed_count;
        if result.conflict_count > 0 {
            output::success(format!(
                "Sync complete ({total} resources, {} conflicts)",
                result.conflict_count
            ));
        } else {
            output::success(format!("Sync complete ({total} resources)"));
        }
    }

    Ok(())
}

/// Show sync status without making changes (dry-run).
pub fn status(contexts: &[String], format: &str) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;

    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let (rt, client) = runtime::build_runtime_and_client()?;

    let diff = rt.block_on(async {
        sync_actions::sync_status_check(&client, &mut manifest, &vault_root, contexts).await
    })?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "to_push": diff.to_push.len(),
            "to_pull": diff.to_pull.len(),
            "conflicts": diff.conflicts.len(),
            "removed": diff.removed.len(),
        });
        output::plain(event);
    } else {
        output::plain(format!(
            "  \u{2191} Push    {} resources",
            diff.to_push.len()
        ));
        output::plain(format!(
            "  \u{2193} Pull    {} resources",
            diff.to_pull.len()
        ));
        output::plain(format!(
            "  \u{2717} Conflict {} resources",
            diff.conflicts.len()
        ));
        output::plain(format!(
            "  \u{2212} Removed {} resources",
            diff.removed.len()
        ));
    }

    Ok(())
}
