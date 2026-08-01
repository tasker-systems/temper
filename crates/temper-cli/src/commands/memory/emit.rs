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

use temper_client::TemperClient;
use temper_core::types::config::{expand_tilde, MemoryConfig, TemperConfig};
use temper_workflow::types::resource::{ResourceDetail, ResourceListParams};

use super::render::{parse_entry, render_index, MemoryDefect};
use crate::actions::runtime::{build_config_store_and_client, client_err_to_temper};
use crate::error::{Result, TemperError};

const MEMORY_DOC_TYPE: &str = "memory";

/// Page size for the `list_meta` walk in [`fetch_context_rows`] — mirrors
/// `status::STATUS_PAGE_SIZE`. Reproduced here (not imported) because that constant and its
/// paging function are private to `status.rs`; the pattern — page to the true `total` rather
/// than accept a capped page — is what's shared, not the code.
const EMIT_PAGE_SIZE: i64 = 200;

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

/// Fetch every `memory`-typed resource in `context_ref`, paging through the full result set.
/// See `status::fetch_context_rows` for why paging to `total` matters — a capped first page
/// would silently drop memories once a context holds more than one page's worth.
async fn fetch_context_rows(
    client: &TemperClient,
    context_ref: &str,
) -> Result<Vec<ResourceDetail>> {
    let mut rows = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let params = ResourceListParams {
            doc_type_name: Some(MEMORY_DOC_TYPE.to_string()),
            context_ref: Some(context_ref.to_string()),
            limit: Some(EMIT_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        };
        let response = client
            .resources()
            .list_meta(&params)
            .await
            .map_err(client_err_to_temper)?;
        let fetched = response.rows.len() as i64;
        rows.extend(response.rows);
        offset += fetched;
        if fetched == 0 || offset >= response.total {
            break;
        }
    }
    Ok(rows)
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

    let path_str = path_override.unwrap_or(mem.index_path.as_str());
    let path = expand_tilde(path_str);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, rendered)?;

    Ok(path)
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
}
