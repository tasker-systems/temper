//! `temper memory status` — the discovery surface.
//!
//! The whole point of this command is that it works on a machine that has
//! never opted into the `[memory]` config section: an absent section is not
//! an error, it is the primary case this command exists to report on.
//!
//! [`status_report`] is pure over its three inputs (config, fetched rows,
//! local files) — no I/O, no clock — so the matching and defect-collection
//! logic is unit-testable without a server or a filesystem. [`status`] is the
//! thin I/O shell: it resolves what to fetch/scan from `config.memory`, calls
//! the server (when opted in), reads local files (when opted in), and prints.

use std::collections::HashSet;

use serde::Serialize;

use temper_client::TemperClient;
use temper_core::types::config::{expand_tilde, MemoryConfig, TemperConfig};
use temper_workflow::types::resource::{ResourceDetail, ResourceListParams};

use super::render::parse_entry;
use crate::actions::runtime::{build_config_store_and_client, client_err_to_temper};
use crate::error::Result;
use crate::format::{self, OutputFormat};
use crate::output;

const MEMORY_DOC_TYPE: &str = "memory";

/// Page size for the `list_meta` walk in [`fetch_context_rows`]. Larger than the CLI's own
/// browsing default (`DEFAULT_META_LIST_LIMIT`, 50, in `commands/resource.rs`) because this
/// walk exists to produce an accurate total for the report, not a page to browse — see the
/// pagination note on `fetch_context_rows`.
const STATUS_PAGE_SIZE: i64 = 200;

/// One `.md` file found in the directory that holds the rendered index (`index_path`'s parent).
/// Carries only the filename — matching against Temper is by filename stem, never a full path,
/// so a report stays legible across machines whose vault lives at different absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMemoryFile {
    pub filename: String,
}

/// This machine's memory state — the discovery report.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStatus {
    /// Whether `[memory]` is configured at all. `false` is not an error state.
    pub opted_in: bool,
    /// Every context this machine would render for, shared first (empty when not opted in).
    pub contexts: Vec<String>,
    /// Count of fetched rows that parsed cleanly into a [`super::render::MemoryEntry`].
    pub in_temper: usize,
    /// One rendered [`super::render::MemoryDefect`] message per row that failed to parse —
    /// reported, never fatal. Only `emit` refuses on a defect.
    pub defects: Vec<String>,
    /// Count of `.md` files found alongside the index (excluding the index file itself).
    pub local_files: usize,
    /// Filenames present locally whose stem (filename minus `.md`) matches no successfully
    /// parsed Temper memory's title. An unadopted machine's local files are all reported here —
    /// that is the point: discovery must say what this machine is carrying even with nothing to
    /// compare it against.
    pub local_without_counterpart: Vec<String>,
}

/// Pure core of `status`. Takes the memory config (`None` = feature off), every fetched
/// `memory`-typed row across all configured contexts, and every local `.md` file found next to
/// the index — and reports, without failing on a malformed row.
///
/// Matching a local file to a Temper memory is by filename stem vs. title: strip the `.md`
/// extension from the local filename and compare against the title of each row that parsed
/// cleanly via [`parse_entry`]. A row that fails to parse contributes to `defects`, never to the
/// matched set — an unparseable memory cannot vouch for a local file's counterpart existing.
pub fn status_report(
    cfg: Option<&MemoryConfig>,
    rows: &[ResourceDetail],
    local: &[LocalMemoryFile],
) -> MemoryStatus {
    let contexts = cfg
        .map(|c| c.all_contexts().into_iter().map(String::from).collect())
        .unwrap_or_default();

    let mut in_temper = 0usize;
    let mut defects = Vec::new();
    let mut known_titles: HashSet<String> = HashSet::new();

    for row in rows {
        match parse_entry(row) {
            Ok(entry) => {
                in_temper += 1;
                known_titles.insert(entry.title);
            }
            Err(defect) => defects.push(defect.to_string()),
        }
    }

    let local_without_counterpart = local
        .iter()
        .filter(|f| {
            let stem = f.filename.strip_suffix(".md").unwrap_or(&f.filename);
            !known_titles.contains(stem)
        })
        .map(|f| f.filename.clone())
        .collect();

    MemoryStatus {
        opted_in: cfg.is_some(),
        contexts,
        in_temper,
        defects,
        local_files: local.len(),
        local_without_counterpart,
    }
}

/// The I/O shell. Reads `config.memory`: `None` means the feature is off, and this still
/// returns `Ok` with a report saying so — an unadopted machine is the primary case, not an
/// error path. When opted in, reads local `.md` files from the directory containing
/// `index_path`, fetches every `memory`-typed resource in each configured context, and prints
/// the resulting [`MemoryStatus`] via [`format::render`] (JSON/TOON per `output_format`).
///
/// Client construction and each per-context fetch degrade gracefully (a warning on stderr, not
/// a hard failure) rather than refusing the whole report — an opted-in-but-unauthenticated or
/// momentarily-unreachable machine still gets a local-files report instead of nothing, matching
/// this command's status as the discovery surface.
pub async fn status(config: &TemperConfig, output_format: OutputFormat) -> Result<()> {
    let mem = config.memory.as_ref();

    let local = mem
        .map(|m| read_local_files(&m.index_path))
        .unwrap_or_default();

    let mut rows: Vec<ResourceDetail> = Vec::new();
    if let Some(mem) = mem {
        match build_config_store_and_client() {
            Ok((_cfg, _store, client)) => {
                for ctx in mem.all_contexts() {
                    match fetch_context_rows(&client, ctx).await {
                        Ok(mut ctx_rows) => rows.append(&mut ctx_rows),
                        Err(e) => {
                            output::warning(format!("could not list memories in {ctx}: {e}"));
                        }
                    }
                }
            }
            Err(e) => {
                output::warning(format!(
                    "cloud unreachable — reporting local files only ({e})"
                ));
            }
        }
    }

    let report = status_report(mem, &rows, &local);
    let rendered = format::render(&report, output_format)?;
    println!("{rendered}");
    Ok(())
}

/// Fetch every `memory`-typed resource in `context_ref`, paging through the full result set.
///
/// `list_meta` returns a capped page (`ResourceMetaListResponse { rows, total, .. }`).
/// Reporting `rows.len()` as the whole count would silently understate `in_temper` and
/// misclassify locally-orphaned files the moment a context holds more memories than one page —
/// status exists to answer "what do I actually have", so it pages to the true total rather than
/// surfacing a partial count as if it were complete.
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
            limit: Some(STATUS_PAGE_SIZE),
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

/// List `.md` files in the directory containing `index_path`, excluding the index file itself.
/// Returns an empty list (never an error) when the directory or its parent is unreachable — a
/// machine mid-adoption whose configured directory does not exist yet still gets a report.
fn read_local_files(index_path: &str) -> Vec<LocalMemoryFile> {
    let expanded = expand_tilde(index_path);
    let Some(dir) = expanded.parent() else {
        return Vec::new();
    };
    let index_filename = expanded
        .file_name()
        .and_then(|f| f.to_str())
        .map(str::to_string);

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files: Vec<LocalMemoryFile> = entries
        .filter_map(std::io::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if !name.ends_with(".md") {
                return None;
            }
            if index_filename.as_deref() == Some(name.as_str()) {
                return None;
            }
            Some(LocalMemoryFile { filename: name })
        })
        .collect();
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use temper_core::types::ids::{ProfileId, ResourceId};
    use temper_workflow::types::resource::{BodyStorage, IngestState, ResourceRow};
    use uuid::Uuid;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            shared_contexts: vec![],
            project_contexts: vec!["@me/temper".to_string()],
            index_path: "~/.claude/projects/p/memory/MEMORY.md".to_string(),
            stale_after_days: 90,
        }
    }

    fn local(filename: &str) -> LocalMemoryFile {
        LocalMemoryFile {
            filename: filename.to_string(),
        }
    }

    /// Build a `ResourceDetail` titled `title`, homed in `context_ref` (`@owner/slug`), with
    /// `open_meta` carrying `status`/`verified` only when the corresponding argument is `Some` —
    /// mirrors `render::tests::row_with`, parameterized by title and context since status must
    /// match across multiple distinct rows.
    fn build_row(
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

    fn meta_row_titled(title: &str, context_ref: &str) -> ResourceDetail {
        build_row(title, context_ref, Some("active"), Some("2026-08-01"))
    }

    fn meta_row_missing_status(title: &str, context_ref: &str) -> ResourceDetail {
        build_row(title, context_ref, None, Some("2026-08-01"))
    }

    #[test]
    fn status_works_when_not_opted_in() {
        let r = status_report(None, &[], &[local("feedback_x.md")]);
        assert!(!r.opted_in);
        assert_eq!(r.local_files, 1);
        assert_eq!(
            r.local_without_counterpart,
            vec!["feedback_x.md"],
            "an unadopted machine must still be told what it is carrying"
        );
    }

    #[test]
    fn status_matches_local_files_to_temper_by_slug() {
        let rows = vec![meta_row_titled("feedback_x", "@me/temper")];
        let r = status_report(
            Some(&cfg()),
            &rows,
            &[local("feedback_x.md"), local("feedback_y.md")],
        );
        assert_eq!(r.in_temper, 1);
        assert_eq!(r.local_without_counterpart, vec!["feedback_y.md"]);
    }

    #[test]
    fn status_reports_defects_without_failing() {
        let rows = vec![meta_row_missing_status("feedback_x", "@me/temper")];
        let r = status_report(Some(&cfg()), &rows, &[]);
        assert_eq!(
            r.defects.len(),
            1,
            "status REPORTS defects; only emit refuses on them"
        );
    }
}
