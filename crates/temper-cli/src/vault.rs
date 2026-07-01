use crate::error::{Result, TemperError};
use crate::templates::{
    ConceptTemplate, DecisionTemplate, GoalTemplate, ResearchTemplate, SessionTemplate,
    TaskTemplate,
};
use askama::Template;
use std::path::Path;
use std::path::PathBuf;
use temper_workflow::frontmatter::DocType;

// ---------------------------------------------------------------------------
// Validation constants and helpers
// ---------------------------------------------------------------------------

pub const VALID_STAGES: &[&str] = &["backlog", "in-progress", "done", "cancelled"];
pub const VALID_MODES: &[&str] = &["plan", "build"];
pub const VALID_EFFORTS: &[&str] = &["small", "medium", "large"];

pub fn validate_stage(s: &str) -> Result<()> {
    if !VALID_STAGES.contains(&s) {
        return Err(TemperError::Vault(format!(
            "invalid stage: {s}. Must be one of: {}",
            VALID_STAGES.join(", ")
        )));
    }
    Ok(())
}

pub fn validate_mode(m: &str) -> Result<()> {
    if !VALID_MODES.contains(&m) {
        return Err(TemperError::Vault(format!(
            "invalid mode: {m}. Must be one of: {}",
            VALID_MODES.join(", ")
        )));
    }
    Ok(())
}

pub fn validate_effort(e: &str) -> Result<()> {
    if !VALID_EFFORTS.contains(&e) {
        return Err(TemperError::Vault(format!(
            "invalid effort: {e}. Must be one of: {}",
            VALID_EFFORTS.join(", ")
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Template rendering (askama-based)
// ---------------------------------------------------------------------------

/// Return a rendered template with placeholder values for a note type.
/// Used by `--show-template` flags to preview template structure.
///
/// Takes a typed [`DocType`] — the string→enum parse happens once at the CLI
/// dispatch boundary, so this match is exhaustive over the owned variant set
/// and a new doctype forces a template to be added here.
pub fn get_template(doc_type: DocType) -> Result<String> {
    let placeholder = "{{placeholder}}";
    match doc_type {
        DocType::Task => TaskTemplate {
            id: placeholder,
            title: placeholder,
            slug: placeholder,
            context: placeholder,
            goal: placeholder,
            mode: "null",
            effort: "null",
            seq: "0",
            datetime: placeholder,
        }
        .render(),
        DocType::Session => SessionTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
        }
        .render(),
        DocType::Goal => GoalTemplate {
            id: placeholder,
            title: placeholder,
            slug: placeholder,
            context: placeholder,
            seq: "0",
            date: placeholder,
        }
        .render(),
        DocType::Research => ResearchTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
            project: placeholder,
            slug: placeholder,
        }
        .render(),
        DocType::Concept => ConceptTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
            project: placeholder,
            slug: placeholder,
        }
        .render(),
        DocType::Decision => DecisionTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
            project: placeholder,
            slug: placeholder,
        }
        .render(),
        // Cognitive-map node labels (spec D3) are born via cogmap ingest
        // (facet_set / reconcile), not via `temper resource create` — they
        // have no CLI file-template. Kept as an explicit arm so a new
        // resource doctype still forces a template decision here.
        DocType::Fact
        | DocType::Memory
        | DocType::Question
        | DocType::Theme
        | DocType::Concern
        | DocType::Principle
        | DocType::Commitment
        | DocType::Domain => {
            return Err(TemperError::Vault(format!(
                "no CLI template for cognitive-map label '{}'; these are created via cogmap ingest, not `temper resource create`",
                doc_type.as_str()
            )));
        }
    }
    .map_err(|e| TemperError::Vault(format!("Failed to render template: {e}")))
}

/// Extract wikilinks from markdown content: [[Link Name]]
pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("[[") {
        let after = &remaining[start + 2..];
        if let Some(end) = after.find("]]") {
            let link = &after[..end];
            // Handle [[actual|display]] Obsidian syntax — target is first segment
            let actual = link.split('|').next().unwrap_or(link).trim();
            links.push(actual.to_string());
            remaining = &after[end + 2..];
        } else {
            break;
        }
    }
    links
}

/// Simple slug generation from a title
pub fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Write a note to the filesystem, creating parent directories as needed
pub fn write_note(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Read stdin content if stdin is not a terminal (piped input).
/// Returns None if stdin is a terminal or if reading fails.
pub fn read_stdin_if_piped() -> Option<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).ok()?;
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// Recursively collect all .md files under a directory
pub fn collect_md_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_files_recursive(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            files.push(path);
        }
    }
    Ok(())
}
