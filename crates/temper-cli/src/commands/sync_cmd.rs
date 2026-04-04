//! `temper sync` — bidirectional sync between local vault and temper cloud.
//!
//! Subcommands:
//! - `temper sync run` — full sync cycle (push, pull, remove, complete)
//! - `temper sync status` — dry-run diff (no changes)
//! - `temper sync refresh` — interleave server manifest into local manifest
//! - `temper sync reset` — rebuild manifest from scratch (backup + full rebuild)

use crate::actions::progress::TerminalProgress;
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

    let progress = TerminalProgress::new();
    let result = rt.block_on(async {
        sync_actions::sync_orchestration(&client, &mut manifest, &vault_root, contexts, &progress)
            .await
    })?;

    // Save manifest after successful sync
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "event": "sync_complete",
            "scanned": result.scan_count,
            "pushed": result.push_count,
            "pulled": result.pull_count,
            "conflicts": result.conflict_count,
            "merge_auto": result.merge_auto_count,
            "merge_conflict": result.merge_conflict_count,
            "removed": result.removed_count,
            "errors": result.error_count,
        });
        output::plain(event);
    } else {
        if result.scan_count > 0 {
            output::plain(format!("  + Scan    {} new files", result.scan_count));
        }
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
                "  \u{21c5} Merge   {} resources ({} auto, {} conflict)",
                result.conflict_count, result.merge_auto_count, result.merge_conflict_count,
            ));
        }
        if result.removed_count > 0 {
            output::plain(format!(
                "  \u{2212} Removed {} resources",
                result.removed_count
            ));
        }
        if result.error_count > 0 {
            output::error(format!(
                "  ! Errors  {} resources failed",
                result.error_count
            ));
        }
        let total = result.push_count + result.pull_count + result.removed_count;
        if result.error_count > 0 {
            output::warning(format!(
                "Sync complete ({total} resources, {} error(s))",
                result.error_count
            ));
        } else if result.merge_conflict_count > 0 {
            output::success(format!(
                "Sync complete ({total} resources, {} merge conflict(s))",
                result.merge_conflict_count
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

    let progress = TerminalProgress::new();
    let diff = rt.block_on(async {
        sync_actions::sync_status_check(&client, &mut manifest, &vault_root, contexts, &progress)
            .await
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

/// Refresh manifest from server — non-destructive interleave.
pub fn refresh(format: &str) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;

    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let (rt, client) = runtime::build_runtime_and_client()?;

    // Ensure profile exists before hitting sync endpoints
    rt.block_on(runtime::ensure_profile(&client))?;

    let result = rt.block_on(async {
        sync_actions::sync_refresh(&client, &mut manifest, &vault_root).await
    })?;

    // Save manifest after successful refresh
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "event": "refresh_complete",
            "matched": result.matched,
            "added": result.added,
            "orphaned": result.orphaned,
            "pending_preserved": result.pending_preserved,
        });
        output::plain(event);
    } else {
        output::plain(format!("  \u{2714} Matched  {} entries", result.matched));
        output::plain(format!("  + Added    {} entries", result.added));
        if result.orphaned > 0 {
            output::warning(format!(
                "  ? Orphaned {} entries (local-only, no server match)",
                result.orphaned
            ));
        }
        if result.pending_preserved > 0 {
            output::plain(format!(
                "  \u{23f3} Pending  {} entries preserved",
                result.pending_preserved
            ));
        }
        output::success("Manifest refresh complete");
    }

    Ok(())
}

/// Reset manifest from scratch — backup + full rebuild.
pub fn reset(format: &str) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;

    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let (rt, client) = runtime::build_runtime_and_client()?;

    // Ensure profile exists before hitting sync endpoints
    rt.block_on(runtime::ensure_profile(&client))?;

    // Backup before reset
    sync_actions::backup_manifest(&temper_dir)?;

    let (new_manifest, result) = rt.block_on(async {
        sync_actions::sync_reset(&client, &manifest, &vault_root).await
    })?;

    // Save rebuilt manifest
    crate::manifest_io::save_manifest(&temper_dir, &new_manifest)?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "event": "reset_complete",
            "matched_by_id": result.matched_by_id,
            "matched_by_hash": result.matched_by_hash,
            "unmatched_local": result.unmatched_local,
            "unmatched_remote": result.unmatched_remote,
        });
        output::plain(event);
    } else {
        output::plain(format!(
            "  \u{2714} Matched  {} by temper-id",
            result.matched_by_id
        ));
        output::plain(format!(
            "  \u{2714} Matched  {} by content hash",
            result.matched_by_hash
        ));
        if result.unmatched_local > 0 {
            output::plain(format!(
                "  + Pending  {} local files (new, will push on next sync)",
                result.unmatched_local
            ));
        }
        if result.unmatched_remote > 0 {
            output::plain(format!(
                "  \u{2193} To pull  {} remote resources (will pull on next sync)",
                result.unmatched_remote
            ));
        }
        output::success("Manifest reset complete (backup saved)");
    }

    Ok(())
}
