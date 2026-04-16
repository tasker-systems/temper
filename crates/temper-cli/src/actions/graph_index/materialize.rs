//! Materialization — writes Concept files and updates member edges.
//!
//! Per-concept: generate `temper-provisional-id` (UUIDv7), write Concept file,
//! add `relates-to` edges to member frontmatter. Transactional per-concept:
//! if any member edge write fails, delete the Concept file.

use std::fs;

use uuid::Uuid;

use temper_core::frontmatter::Frontmatter;
use temper_core::types::config::GraphIndexConfig;
use temper_core::types::ManagedMeta;
use temper_llm::types::ConceptProposal;

use crate::config::Config;

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

        let slug = proposal.slug.as_ref().unwrap();
        let title = proposal.title.as_ref().unwrap_or(slug);
        let body = proposal.body_markdown.as_ref().unwrap_or(&String::new());

        // Generate IDs
        let provisional_id = Uuid::new_v7().to_string();
        let llm_run = Uuid::new_v4().to_string();

        // Build Concept frontmatter
        let concept_path = vault_root
            .join("@me")
            .join("temper")
            .join("concept")
            .join(format!("{}.md", slug));

        // For dry run, just count and continue
        if dry_run {
            report.concepts_created += 1;
            report.members_updated += proposal.member_edges.len();
            continue;
        }

        // Write Concept file
        let concept_content = build_concept_content(slug, title, body, &provisional_id, &llm_run, "llm-discovered");
        if let Err(e) = fs::create_dir_all(concept_path.parent().unwrap()) {
            report.errors += 1;
            report.failed.push(format!("create concept dir: {}", e));
            continue;
        }
        if let Err(e) = fs::write(&concept_path, &concept_content) {
            report.errors += 1;
            report.failed.push(format!("write concept {}: {}", slug, e));
            continue;
        }

        // Update member edges
        let mut concept_file_deleted = false;
        for edge in &proposal.member_edges {
            let member_path = find_member_path(vault_root, &edge.target_slug);
            if let Some(member_path) = member_path {
                match add_relates_to_edge(&member_path, slug, &graph_config.concept_default_edge_type) {
                    Ok(_) => report.members_updated += 1,
                    Err(e) => {
                        // Roll back: delete the Concept file
                        let _ = fs::remove_file(&concept_path);
                        concept_file_deleted = true;
                        report.errors += 1;
                        report.failed.push(format!("member edge {}: {}", edge.target_slug, e));
                        break;
                    }
                }
            } else {
                report.errors += 1;
                report.failed.push(format!("member not found: {}", edge.target_slug));
            }
        }

        if !concept_file_deleted {
            report.concepts_created += 1;
        }
    }

    report
}

fn build_concept_content(
    slug: &str,
    title: &str,
    body: &str,
    provisional_id: &str,
    llm_run: &str,
    provenance: &str,
) -> String {
    format!(
        "---\ntemper-id: {}\ntemper-provisional-id: {}\ntemper-provenance: {}\ntemper-llm-run: {}\ntags:\n  - concept\n---\n\n# {}\n\n{}",
        Uuid::new_v7().to_string(),
        provisional_id,
        provenance,
        llm_run,
        title,
        body
    )
}

/// Find a member's file path by slug.
fn find_member_path(vault_root: &std::path::Path, slug: &str) -> Option<std::path::PathBuf> {
    let search_dirs = ["@me/temper/task", "@me/temper/goal", "@me/temper/session", "@me/temper/research"];
    for dir in search_dirs {
        let path = vault_root.join(dir).join(format!("{}.md", slug));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Add a relates-to edge to a member's frontmatter.
fn add_relates_to_edge(
    path: &std::path::Path,
    target_slug: &str,
    edge_type: &str,
) -> std::result::Result<(), String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let frontmatter = Frontmatter::try_from(raw.as_str()).map_err(|e| e.to_string())?;

    // Add relates-to to open_meta
    let mut open_meta = frontmatter.open_meta.unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = open_meta.as_object_mut() {
        obj.entry("relates-to")
            .or_insert_with(|| serde_json::Value::Array(vec![]));
        if let Some(arr) = obj.get_mut("relates-to").and_then(|v| v.as_array_mut()) {
            let entry = serde_json::json!({
                "slug": target_slug,
                "edge_type": edge_type,
            });
            if !arr.iter().any(|v| v.get("slug").and_then(|s| s.as_str()) == Some(target_slug)) {
                arr.push(entry);
            }
        }
    }

    // Reconstruct the file
    let new_raw = format_frontmatter(&frontmatter, &raw);
    fs::write(path, new_raw).map_err(|e| e.to_string())
}

/// Format frontmatter back into the raw string.
fn format_frontmatter(fm: &Frontmatter, original: &str) -> String {
    // Simple: serialize the frontmatter back to YAML
    let fm_yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    // Find the end of the frontmatter in original
    let frontmatter_end = original.find("---\n").map(|p| p + 4).unwrap_or(0);
    let body = &original[frontmatter_end..];
    format!("---\n{}---\n{}", fm_yaml.trim(), body.trim())
}

/// Report from materialization.
#[derive(Debug, Default)]
pub struct MaterializeReport {
    pub concepts_created: usize,
    pub concepts_skipped: usize,
    pub members_updated: usize,
    pub errors: usize,
    pub failed: Vec<String>,
}