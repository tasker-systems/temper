//! Materialization — writes Concept files and updates member edges.
//!
//! Per-concept: generate `temper-provisional-id` (UUIDv7), write Concept file,
//! append the concept slug to each member's `relates_to` list. Transactional
//! per-concept: if any member edge write fails, the Concept file is removed.

use std::fs;
use std::path::Path;

use serde_yaml::Value;
use uuid::Uuid;

use temper_core::frontmatter::{DocType, Frontmatter};
use temper_core::types::config::GraphIndexConfig;
use temper_llm::types::ConceptProposal;

use crate::config::Config;

/// Report from materialization.
#[derive(Debug, Default, Clone)]
pub struct MaterializeReport {
    pub concepts_created: usize,
    pub concepts_skipped: usize,
    pub members_updated: usize,
    pub errors: usize,
    pub failed: Vec<String>,
}

/// Materialize concept proposals into vault files and member edge updates.
pub fn materialize_concepts(
    proposals: &[ConceptProposal],
    config: &Config,
    graph_config: &GraphIndexConfig,
    dry_run: bool,
) -> MaterializeReport {
    let vault_root = &config.vault_root;
    let mut report = MaterializeReport::default();

    for proposal in proposals {
        if !proposal.is_concept {
            report.concepts_skipped += 1;
            continue;
        }

        let Some(slug) = proposal.slug.as_deref() else {
            report.concepts_skipped += 1;
            continue;
        };
        let title = proposal.title.as_deref().unwrap_or(slug);
        let body = proposal.body_markdown.as_deref().unwrap_or("");

        // Provisional id for pre-sync files; server assigns canonical temper-id on first sync.
        let provisional_id = Uuid::now_v7().to_string();
        let llm_run = Uuid::now_v7().to_string();

        let concept_path = vault_root
            .join("@me")
            .join("temper")
            .join("concept")
            .join(format!("{slug}.md"));

        if dry_run {
            report.concepts_created += 1;
            report.members_updated += proposal.member_edges.len();
            continue;
        }

        let concept_content =
            match build_concept_content(slug, title, body, &provisional_id, &llm_run) {
                Ok(c) => c,
                Err(e) => {
                    report.errors += 1;
                    report.failed.push(format!("build concept {slug}: {e}"));
                    continue;
                }
            };

        if let Some(parent) = concept_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                report.errors += 1;
                report.failed.push(format!("create concept dir: {e}"));
                continue;
            }
        }
        if let Err(e) = fs::write(&concept_path, &concept_content) {
            report.errors += 1;
            report.failed.push(format!("write concept {slug}: {e}"));
            continue;
        }

        let mut rolled_back = false;
        for edge in &proposal.member_edges {
            // `target_path` is the canonical `@me/{context}/{doctype}/{file}.md`
            // rel_path that the LLM was handed in the judgment prompt — resolve
            // directly against `vault_root`. No probing across doctype dirs.
            let member_path = vault_root.join(&edge.target_path);
            if !member_path.exists() {
                report.errors += 1;
                report
                    .failed
                    .push(format!("member not found: {}", edge.target_path));
                continue;
            }

            match append_edge(&member_path, slug, &graph_config.concept_default_edge_type) {
                Ok(()) => report.members_updated += 1,
                Err(e) => {
                    let _ = fs::remove_file(&concept_path);
                    rolled_back = true;
                    report.errors += 1;
                    report
                        .failed
                        .push(format!("member edge {} -> {slug}: {e}", edge.target_path));
                    break;
                }
            }
        }

        if !rolled_back {
            report.concepts_created += 1;
        }
    }

    report
}

/// Build the canonical Concept file content via `Frontmatter::new` + typed setters.
fn build_concept_content(
    slug: &str,
    title: &str,
    body: &str,
    provisional_id: &str,
    llm_run: &str,
) -> Result<String, String> {
    let mut fm = Frontmatter::new(DocType::Concept, format!("\n# {title}\n\n{body}\n"));
    fm.set_managed_field(
        "temper-provisional-id",
        serde_json::Value::String(provisional_id.to_string()),
    );
    fm.set_open_field("slug", serde_json::Value::String(slug.to_string()));
    fm.set_open_field("title", serde_json::Value::String(title.to_string()));
    fm.set_open_field(
        "date",
        serde_json::Value::String(chrono::Utc::now().format("%Y-%m-%d").to_string()),
    );
    fm.set_open_field(
        "temper-provenance",
        serde_json::Value::String("llm-discovered".to_string()),
    );
    fm.set_open_field(
        "temper-llm-run",
        serde_json::Value::String(llm_run.to_string()),
    );
    fm.set_open_field(
        "tags",
        serde_json::Value::Array(vec![serde_json::Value::String("concept".to_string())]),
    );
    fm.serialize().map_err(|e| e.to_string())
}

/// Append the concept slug to the member's edge list (default bucket: `relates_to`).
///
/// The bucket key comes from `graph_config.concept_default_edge_type` (canonical
/// YAML key form, e.g. `"relates-to"` or `"relates_to"` — `Frontmatter::parse_file`
/// alias-normalizes on read, so we always write to the canonical underscore form).
fn append_edge(path: &Path, concept_slug: &str, edge_type: &str) -> Result<(), String> {
    let mut fm = Frontmatter::parse_file(path).map_err(|e| e.to_string())?;
    let key = canonicalize_edge_key(edge_type);

    let mapping = fm
        .value_mut()
        .as_mapping_mut()
        .ok_or_else(|| "frontmatter is not a mapping".to_string())?;

    let key_value = Value::String(key.to_string());
    let existing = mapping
        .get(&key_value)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if existing.iter().any(|s| s == concept_slug) {
        return Ok(());
    }

    let mut merged: Vec<Value> = existing.into_iter().map(Value::String).collect();
    merged.push(Value::String(concept_slug.to_string()));
    mapping.insert(key_value, Value::Sequence(merged));

    fm.write_to(path).map_err(|e| e.to_string())
}

/// Hyphen→underscore conversion so user-configured `"relates-to"` maps onto the
/// canonical `relates_to` key that `Frontmatter::parse_file` normalizes to.
fn canonicalize_edge_key(edge_type: &str) -> String {
    edge_type.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use temper_llm::types::MemberEdge;

    /// `materialize_concepts` must resolve a `MemberEdge.target_path` directly
    /// via `vault_root.join(rel_path)` — no probing through hard-coded
    /// doctype subdirs. This exercises a member at `@me/temper/task/...` with
    /// a date-prefixed filename, which `find_member_path`'s `slug.md` probing
    /// would miss.
    #[test]
    fn test_materialize_concepts_looks_up_member_by_rel_path() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_root = tmp.path();

        // A realistic member path — date-prefixed filename, so no slug probe
        // ending in `{slug}.md` could ever find it.
        let member_rel =
            "@me/temper/task/2026-04-11-resource-update-should-accept-stdin-for-body-content.md";
        let member_full = vault_root.join(member_rel);
        fs::create_dir_all(member_full.parent().unwrap()).unwrap();
        fs::write(
            &member_full,
            "---\n\
temper-id: \"01900000-0000-7000-8000-000000000001\"\n\
temper-type: task\n\
temper-context: temper\n\
temper-created: \"2026-01-01T00:00:00Z\"\n\
temper-owner: \"@me\"\n\
title: \"Resource Update Accepts Stdin\"\n\
temper-stage: backlog\n\
slug: resource-update-accepts-stdin\n\
---\n\
\n\
body\n",
        )
        .unwrap();

        let config = Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: Vec::new(),
            skill_output: vault_root.join(".skill"),
        };
        let graph_config = GraphIndexConfig {
            concept_default_edge_type: "relates-to".to_string(),
            ..GraphIndexConfig::default()
        };

        let proposal = ConceptProposal {
            is_concept: true,
            slug: Some("stdin-body-input".to_string()),
            title: Some("Stdin Body Input".to_string()),
            body_markdown: Some("## Members\n".to_string()),
            member_edges: vec![MemberEdge {
                target_path: member_rel.to_string(),
                edge_type: "relates-to".to_string(),
            }],
        };

        let report = materialize_concepts(&[proposal], &config, &graph_config, false);

        assert_eq!(
            report.errors, 0,
            "no errors; failed messages: {:?}",
            report.failed
        );
        assert_eq!(report.concepts_created, 1, "concept created");
        assert_eq!(
            report.members_updated, 1,
            "member edge added; failed: {:?}",
            report.failed
        );

        let fm = Frontmatter::parse_file(&member_full).unwrap();
        let relates: Vec<String> = fm
            .value()
            .get("relates_to")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            relates.iter().any(|s| s == "stdin-body-input"),
            "member has relates_to edge to concept slug; got {relates:?}"
        );
    }
}
