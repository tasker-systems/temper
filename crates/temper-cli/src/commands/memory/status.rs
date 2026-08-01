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

use temper_core::types::config::{expand_tilde, MemoryConfig, TemperConfig};
use temper_workflow::types::resource::ResourceDetail;

use super::fetch::fetch_context_rows;
use super::render::parse_entry;
use crate::actions::runtime::build_config_store_and_client;
use crate::error::Result;
use crate::format::{self, OutputFormat};
use crate::output;

/// One `.md` file found in the directory that holds the rendered index (`index_path`'s parent).
/// Carries only the filename — matching against Temper is against `open_meta.source_file`
/// (an exact filename, never a full path), so a report stays legible across machines whose vault
/// lives at different absolute paths.
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
    /// Filenames present locally that no fetched row's `open_meta.source_file` names. An
    /// unadopted machine's local files are all reported here — that is the point: discovery must
    /// say what this machine is carrying even with nothing to compare it against. A memory whose
    /// `source_file` is absent (authored natively in Temper — from a session, from Desktop) is
    /// ordinary and simply does not contribute a match; it is never itself reported as orphaned,
    /// since orphaning is a property of local files, not of Temper resources.
    pub local_without_counterpart: Vec<String>,
}

/// Extract `open_meta.source_file` from a row, if present. Independent of whether the row parses
/// cleanly via [`parse_entry`] — a memory with a malformed `status`/`verified` is still a real
/// migrated file and should still vouch for its local counterpart; that a memory contributes a
/// defect and that it contributes provenance are orthogonal facts about it.
fn source_file_of(row: &ResourceDetail) -> Option<String> {
    row.open_meta
        .as_ref()?
        .get("source_file")?
        .as_str()
        .map(str::to_owned)
}

/// Pure core of `status`. Takes the memory config (`None` = feature off), every fetched
/// `memory`-typed row across all configured contexts, and every local `.md` file found next to
/// the index — and reports, without failing on a malformed row.
///
/// Matching a local file to a Temper memory is by exact filename against `open_meta.source_file`
/// — never a title or a sluggified/derived form. A migrated memory's human-readable title
/// ("a clause cannot retire its own goal") is never its filename
/// ("feedback_a_clause_cannot_retire_its_own_goal.md"), so any comparison through title would
/// misreport every migrated memory's local file as orphaned. `source_file` is optional (a memory
/// authored natively in Temper never had one), so its absence on a given row simply means that
/// row contributes no match — never a defect, and never a false orphan for some unrelated file.
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
    let mut known_source_files: HashSet<String> = HashSet::new();

    for row in rows {
        if let Some(source_file) = source_file_of(row) {
            known_source_files.insert(source_file);
        }
        match parse_entry(row) {
            Ok(_) => in_temper += 1,
            Err(defect) => defects.push(defect.to_string()),
        }
    }

    let local_without_counterpart = local
        .iter()
        .filter(|f| !known_source_files.contains(&f.filename))
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
        source_file: Option<&str>,
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
        if let Some(sf) = source_file {
            open.insert("source_file".to_string(), json!(sf));
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
        build_row(title, context_ref, Some("active"), Some("2026-08-01"), None)
    }

    fn meta_row_missing_status(title: &str, context_ref: &str) -> ResourceDetail {
        build_row(title, context_ref, None, Some("2026-08-01"), None)
    }

    /// A memory migrated from a local `.md` file — `title` is the human-readable hook, distinct
    /// from `source_file`, the filename it was migrated from. This is the real-world shape: see
    /// `status_matches_local_files_by_source_file_not_title` for why a title/filename comparison
    /// cannot stand in for this.
    fn meta_row_migrated(title: &str, context_ref: &str, source_file: &str) -> ResourceDetail {
        build_row(
            title,
            context_ref,
            Some("active"),
            Some("2026-08-01"),
            Some(source_file),
        )
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

    /// The real-world shape: a migrated memory's title is a human-readable hook
    /// ("a clause cannot retire its own goal"), never the filename stem
    /// ("feedback_a_clause_cannot_retire_its_own_goal"). Matching must go through
    /// `open_meta.source_file` — a title/filename-stem comparison would report BOTH local files
    /// as orphaned here, not just the genuinely unmatched one.
    #[test]
    fn status_matches_local_files_by_source_file_not_title() {
        let rows = vec![meta_row_migrated(
            "a clause cannot retire its own goal",
            "@me/temper",
            "feedback_a_clause_cannot_retire_its_own_goal.md",
        )];
        let r = status_report(
            Some(&cfg()),
            &rows,
            &[
                local("feedback_a_clause_cannot_retire_its_own_goal.md"),
                local("feedback_y.md"),
            ],
        );
        assert_eq!(r.in_temper, 1);
        assert_eq!(
            r.local_without_counterpart,
            vec!["feedback_y.md"],
            "matching is by source_file; a migrated memory's title never equals its filename"
        );
    }

    /// A memory authored natively in Temper (from a session, from Desktop) never had a source
    /// file. That absence is ordinary — never a defect, and never grounds to treat some unrelated
    /// local file as matched or orphaned by accident.
    #[test]
    fn a_memory_without_a_source_file_is_ordinary_not_a_defect() {
        let native = meta_row_titled("a native memory", "@me/temper");
        let r = status_report(Some(&cfg()), &[native], &[local("feedback_z.md")]);
        assert_eq!(r.in_temper, 1);
        assert!(
            r.defects.is_empty(),
            "a memory with no source_file must not become a defect"
        );
        assert_eq!(
            r.local_without_counterpart,
            vec!["feedback_z.md"],
            "an unrelated local file stays orphaned; a source-file-less memory matches nothing"
        );
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
