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

/// Emit a human-readable warning listing the first few blocked paths from a
/// normalize pass. Used by sync subcommands to surface schema violations the
/// user needs to fix before those files can be synced.
fn warn_blocked_paths(report: &sync_actions::NormalizeReport) {
    if report.blocked == 0 {
        return;
    }
    output::warning(format!(
        "{} file(s) have schema violations and will be skipped from sync:",
        report.blocked
    ));
    for (path, issues) in report.issues_by_path.iter().take(5) {
        let first_message = issues
            .first()
            .map(|i| i.message.as_str())
            .unwrap_or("unknown issue");
        output::warning(format!("  {path} — {first_message}"));
    }
    if report.issues_by_path.len() > 5 {
        output::warning(format!(
            "  ... and {} more (run `temper doctor` for the full list)",
            report.issues_by_path.len() - 5
        ));
    }
}

/// Run a full sync cycle.
pub fn run(contexts: &[String], format: &str) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;

    let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    // Phase A invariant: normalize every manifest entry's file before any
    // other sync logic. Per-entry atomic save ensures an interrupt loses at
    // most one file's work.
    let progress = TerminalProgress::new();
    let normalize_report = sync_actions::normalize_all_entries(
        &mut manifest,
        &vault_root,
        &temper_dir,
        Some(&progress),
    )?;
    warn_blocked_paths(&normalize_report);

    // Preflight: detect and warn about ownership mismatches.
    let ownership_mismatches = sync_actions::preflight_ownership_check(&manifest, &vault_root);
    if !ownership_mismatches.is_empty() {
        output::warning(format!(
            "{} file(s) have ownership mismatches and will be skipped from upload:",
            ownership_mismatches.len()
        ));
        for m in &ownership_mismatches {
            output::warning(format!(
                "  {} — frontmatter: {}, manifest: {}",
                m.file_path, m.frontmatter_owner, m.manifest_owner
            ));
        }
        output::hint(
            "Ownership transfers require an explicit server action (not yet implemented). \
             Revert the frontmatter edit or wait for `temper team transfer`.",
        );
    }

    let mut mismatch_paths: std::collections::HashSet<String> = ownership_mismatches
        .iter()
        .map(|m| m.file_path.clone())
        .collect();
    // Blocked-by-normalize entries are also excluded from the push set —
    // sync must never ship a file with unresolved schema violations.
    for (path, _) in &normalize_report.issues_by_path {
        mismatch_paths.insert(path.clone());
    }

    let (rt, client) = runtime::build_runtime_and_client()?;

    // Ensure profile exists before hitting sync endpoints
    rt.block_on(runtime::ensure_profile(&client))?;

    let result = rt.block_on(async {
        sync_actions::sync_orchestration(
            &client,
            &mut manifest,
            &vault_root,
            contexts,
            &progress,
            &mismatch_paths,
        )
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
            "normalized_rewritten": normalize_report.rewritten,
            "normalized_blocked": normalize_report.blocked,
            "normalized_missing": normalize_report.missing,
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

    // Phase A invariant: normalize every entry before any other logic.
    let progress = TerminalProgress::new();
    let normalize_report = sync_actions::normalize_all_entries(
        &mut manifest,
        &vault_root,
        &temper_dir,
        Some(&progress),
    )?;
    warn_blocked_paths(&normalize_report);

    // Preflight: surface ownership mismatches in the status diff.
    let ownership_mismatches = sync_actions::preflight_ownership_check(&manifest, &vault_root);

    let (rt, client) = runtime::build_runtime_and_client()?;

    let diff = rt.block_on(async {
        sync_actions::sync_status_check(&client, &mut manifest, &vault_root, contexts, &progress)
            .await
    })?;

    // Persist rehashed manifest so computed managed/open hashes are retained.
    crate::manifest_io::save_manifest(&temper_dir, &manifest)?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "to_push": diff.to_push.len(),
            "to_pull": diff.to_pull.len(),
            "conflicts": diff.conflicts.len(),
            "removed": diff.removed.len(),
            "ownership_mismatches": ownership_mismatches.len(),
            "normalized_rewritten": normalize_report.rewritten,
            "normalized_blocked": normalize_report.blocked,
            "normalized_missing": normalize_report.missing,
        });
        output::plain(event);
    } else {
        if !ownership_mismatches.is_empty() {
            output::header("Ownership Mismatches");
            for m in &ownership_mismatches {
                output::warning(format!(
                    "  {} — frontmatter: {}, manifest: {}",
                    m.file_path, m.frontmatter_owner, m.manifest_owner
                ));
            }
            output::blank();
        }
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

    // Phase A invariant: normalize every entry before interleaving the
    // server manifest. Ensures the local side is clean before the merge.
    let progress = TerminalProgress::new();
    let normalize_report = sync_actions::normalize_all_entries(
        &mut manifest,
        &vault_root,
        &temper_dir,
        Some(&progress),
    )?;
    warn_blocked_paths(&normalize_report);

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
            "normalized_rewritten": normalize_report.rewritten,
            "normalized_blocked": normalize_report.blocked,
            "normalized_missing": normalize_report.missing,
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

    let (mut new_manifest, result) =
        rt.block_on(async { sync_actions::sync_reset(&client, &manifest, &vault_root).await })?;

    // Phase A invariant: normalize every entry on the freshly rebuilt
    // manifest so its hashes reflect the canonical on-disk form. This is
    // called after rebuild so the new manifest's entries exist to iterate.
    let progress = TerminalProgress::new();
    let normalize_report = sync_actions::normalize_all_entries(
        &mut new_manifest,
        &vault_root,
        &temper_dir,
        Some(&progress),
    )?;
    warn_blocked_paths(&normalize_report);

    // Save rebuilt manifest (normalize already persisted per-entry, but
    // this final save is a belt-and-suspenders no-op).
    crate::manifest_io::save_manifest(&temper_dir, &new_manifest)?;

    if fmt == OutputFormat::Json {
        let event = serde_json::json!({
            "event": "reset_complete",
            "matched_by_id": result.matched_by_id,
            "matched_by_hash": result.matched_by_hash,
            "unmatched_local": result.unmatched_local,
            "unmatched_remote": result.unmatched_remote,
            "normalized_rewritten": normalize_report.rewritten,
            "normalized_blocked": normalize_report.blocked,
            "normalized_missing": normalize_report.missing,
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
