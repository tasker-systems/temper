//! `temper memory emit` — render the index from Temper and write it.
//!
//! [`build_index`] is the pure core: it maps `parse_entry` over every fetched row, **collecting
//! every defect rather than stopping at the first** — one fix-run should not require N emit-runs
//! to discover N defects — and only calls `render_index` once every row parses cleanly. This is
//! the ONE enforcement point in the system for `open_meta.status` / `open_meta.verified`: those
//! keys live in the open metadata tier, which nothing validates at write time (see
//! `render`'s module doc). `open_meta.source_file` is never validated here — it is optional by
//! design (a memory authored natively in Temper never had one) and requiring it would turn every
//! such memory into a defect.
//!
//! [`emit_outcome`] is a second pure function: whether `[memory]` is configured at all decides,
//! before any I/O, whether this command has anything to do. Absent config is not an error — a
//! no-op that says why, mirroring `status`'s treatment of the same absent-config case.
//!
//! [`emit`] is the async I/O shell: fetch every configured context's `memory`-typed rows, run
//! them through [`build_index`], and write the result to `index_path` (or the override),
//! creating parent directories as needed.

use std::path::PathBuf;

use chrono::{NaiveDate, Utc};

use temper_core::types::config::{expand_tilde, MemoryConfig, TemperConfig};
use temper_workflow::types::resource::ResourceDetail;

use super::fetch::fetch_context_rows;
use super::render::{parse_entry, render_index, MemoryDefect};
use crate::actions::runtime::build_config_store_and_client;
use crate::error::{Result, TemperError};

/// The outcome of checking whether `[memory]` is configured — decided purely, before any I/O, so
/// the CLI dispatch can short-circuit to a no-op-that-explains-itself without ever touching the
/// network or the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmitOutcome {
    /// No `[memory]` section in config.toml. Not an error — an unadopted machine explaining
    /// itself, same treatment as `status`'s `opted_in: false`.
    NotConfigured { reason: String },
    /// `[memory]` is configured; proceed to fetch, build, and write.
    Configured,
}

/// Pure gate: is the memory projection turned on for this machine?
pub fn emit_outcome(mem: Option<&MemoryConfig>) -> EmitOutcome {
    match mem {
        None => EmitOutcome::NotConfigured {
            reason: "no [memory] section in config.toml — the memory projection is off; see `temper memory status`"
                .to_string(),
        },
        Some(_) => EmitOutcome::Configured,
    }
}

/// Map every row through [`parse_entry`], collecting **every** defect rather than
/// short-circuiting on the first, and render the index only once every row parses cleanly.
pub fn build_index(
    cfg: &MemoryConfig,
    rows: &[ResourceDetail],
    today: NaiveDate,
) -> Result<String> {
    let mut entries = Vec::with_capacity(rows.len());
    let mut defects: Vec<MemoryDefect> = Vec::new();

    for row in rows {
        match parse_entry(row) {
            Ok(entry) => entries.push(entry),
            Err(defect) => defects.push(defect),
        }
    }

    if !defects.is_empty() {
        let joined = defects
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(TemperError::BadRequest(format!(
            "{} memor{} failed to parse — fix these before emitting:\n{joined}",
            defects.len(),
            if defects.len() == 1 { "y" } else { "ies" },
        )));
    }

    Ok(render_index(&entries, today, cfg.stale_after_days))
}

/// Resolve the write path (`path_override`, or `mem.index_path` tilde-expanded), create any
/// missing parent directories, and write `rendered` to it.
///
/// Synchronous and network-free by construction — split out from [`emit`] specifically so the
/// half of this command whose entire job is putting bytes on disk is testable with a real
/// filesystem (`tempfile`) without standing up or mocking a client, mirroring
/// `commands/status.rs`'s `count_projected_md_files` tests.
fn resolve_and_write(
    mem: &MemoryConfig,
    path_override: Option<&str>,
    rendered: &str,
) -> Result<PathBuf> {
    let path_str = path_override.unwrap_or(mem.index_path.as_str());
    let path = expand_tilde(path_str);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, rendered)?;
    Ok(path)
}

/// The async I/O shell. Assumes the caller has already checked [`emit_outcome`] (the CLI
/// dispatch does), but re-checks defensively rather than trusting the caller silently.
pub async fn emit(config: &TemperConfig, path_override: Option<&str>) -> Result<PathBuf> {
    let mem = config.memory.as_ref().ok_or_else(|| {
        TemperError::Config(
            "no [memory] section in config.toml — the memory projection is off".to_string(),
        )
    })?;

    let (_cfg, _store, client) = build_config_store_and_client()?;

    let mut rows: Vec<ResourceDetail> = Vec::new();
    for ctx in mem.all_contexts() {
        let mut ctx_rows = fetch_context_rows(&client, ctx).await?;
        rows.append(&mut ctx_rows);
    }

    let today = Utc::now().date_naive();
    let rendered = build_index(mem, &rows, today)?;

    resolve_and_write(mem, path_override, &rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use serde_json::json;
    use temper_core::types::ids::{ProfileId, ResourceId};
    use temper_workflow::types::resource::{BodyStorage, IngestState, ResourceRow};
    use uuid::Uuid;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            shared_contexts: vec![],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/.claude/projects/p/memory/MEMORY.md".to_string(),
            stale_after_days: 90,
        }
    }

    /// Build a `ResourceDetail` titled `title`, homed in `context_ref` (`@owner/slug`), with
    /// `open_meta` carrying `status`/`verified` only when the corresponding argument is `Some` —
    /// mirrors `render::tests::row_with` / `status::tests::build_row`.
    fn row(
        title: &str,
        context_ref: &str,
        status: Option<&str>,
        verified: Option<&str>,
    ) -> ResourceDetail {
        let (owner, slug) = context_ref
            .split_once('/')
            .expect("context_ref must be @owner/slug");

        let mut open = serde_json::Map::new();
        if let Some(s) = status {
            open.insert("status".to_string(), json!(s));
        }
        if let Some(v) = verified {
            open.insert("verified".to_string(), json!(v));
        }

        let row = ResourceRow {
            id: ResourceId::from(Uuid::now_v7()),
            kb_context_id: None,
            origin_uri: String::new(),
            title: title.to_string(),
            originator_profile_id: ProfileId::from(Uuid::now_v7()),
            owner_profile_id: ProfileId::from(Uuid::now_v7()),
            is_active: true,
            created: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            updated: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch"),
            context_name: None,
            doc_type_name: "memory".to_string(),
            owner_handle: "someone".to_string(),
            context_slug: Some(slug.to_string()),
            context_owner_ref: Some(owner.to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            ingest_state: Some(IngestState::Complete),
            body_storage: Some(BodyStorage::Derived),
        };

        ResourceDetail {
            row,
            managed_meta: None,
            open_meta: Some(serde_json::Value::Object(open)),
        }
    }

    fn meta_row_missing_status(title: &str, context_ref: &str) -> ResourceDetail {
        row(title, context_ref, None, Some("2026-08-01"))
    }

    fn meta_row_missing_verified(title: &str, context_ref: &str) -> ResourceDetail {
        row(title, context_ref, Some("active"), None)
    }

    /// A memory authored natively in Temper (from a session, from Desktop) never had a
    /// `source_file` — `row()` deliberately doesn't set one. Mirrors
    /// `status::tests::meta_row_titled` / `a_memory_without_a_source_file_is_ordinary_not_a_defect`.
    fn meta_row_native(title: &str, context_ref: &str) -> ResourceDetail {
        row(title, context_ref, Some("active"), Some("2026-08-01"))
    }

    #[test]
    fn emit_refuses_when_any_memory_is_malformed() {
        let rows = vec![meta_row_missing_status("feedback_x", "@me/temper")];
        let err = build_index(&cfg(), &rows, d("2026-08-01")).expect_err("must refuse");
        assert!(err.to_string().contains("open_meta.status is missing"));
        assert!(
            err.to_string().contains("feedback_x"),
            "the error must name the offending memory"
        );
    }

    #[test]
    fn emit_reports_every_defect_not_just_the_first() {
        let rows = vec![
            meta_row_missing_status("a", "@me/temper"),
            meta_row_missing_verified("b", "@me/temper"),
        ];
        let err = build_index(&cfg(), &rows, d("2026-08-01")).expect_err("must refuse");
        assert!(
            err.to_string().contains("\"a\"") && err.to_string().contains("\"b\""),
            "one fix-run should not require N emit-runs to discover N defects"
        );
    }

    #[test]
    fn emit_is_a_noop_that_explains_itself_when_not_opted_in() {
        let outcome = emit_outcome(None);
        assert!(
            matches!(outcome, EmitOutcome::NotConfigured { .. }),
            "absent [memory] means OFF, and the command must say why rather than erroring"
        );
    }

    /// `open_meta.source_file` is OPTIONAL and must NEVER be required — a memory authored
    /// natively in Temper never had one. This guards the constraint directly: a row with valid
    /// `status`/`verified` and no `source_file` must still build successfully. Without this test
    /// nothing in the suite calls `build_index` on a row that actually succeeds (every existing
    /// row is deliberately defective on `status`/`verified`), so a future change that
    /// re-introduces a `source_file` requirement would pass every other test in this file.
    #[test]
    fn build_index_succeeds_for_a_memory_with_no_source_file() {
        let rows = vec![meta_row_native("a native memory", "@me/temper")];
        let out = build_index(&cfg(), &rows, d("2026-08-01"))
            .expect("a memory with no source_file must build cleanly, never be a defect");
        assert!(out.contains("a native memory"));
    }

    #[test]
    fn resolve_and_write_creates_parent_dirs_and_writes_to_index_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let index_path = dir.path().join("nested/deeper/MEMORY.md");
        let mut c = cfg();
        c.index_path = index_path.to_string_lossy().to_string();

        let written = resolve_and_write(&c, None, "hello index").unwrap();

        assert_eq!(written, index_path);
        assert_eq!(std::fs::read_to_string(&index_path).unwrap(), "hello index");
    }

    #[test]
    fn resolve_and_write_honors_path_override_over_configured_index_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let override_path = dir.path().join("override/OTHER.md");
        let c = cfg();

        let written = resolve_and_write(&c, Some(override_path.to_str().unwrap()), "hi").unwrap();

        assert_eq!(
            written, override_path,
            "an explicit --path must win over the configured index_path"
        );
        assert_eq!(std::fs::read_to_string(&override_path).unwrap(), "hi");
    }

    #[test]
    fn resolve_and_write_overwrites_an_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let index_path = dir.path().join("MEMORY.md");
        std::fs::write(&index_path, "stale content").unwrap();
        let mut c = cfg();
        c.index_path = index_path.to_string_lossy().to_string();

        resolve_and_write(&c, None, "fresh content").unwrap();

        assert_eq!(
            std::fs::read_to_string(&index_path).unwrap(),
            "fresh content",
            "emit re-renders the whole index every run; a stale file must not survive"
        );
    }
}
