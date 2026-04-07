use crate::error::{Result, TemperError};
use crate::templates::{
    ConceptTemplate, DecisionTemplate, GoalTemplate, ResearchTemplate, SessionTemplate,
    TaskTemplate,
};
use askama::Template;
use std::path::Path;
use std::path::PathBuf;

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
pub fn get_template(note_type: &str) -> Result<String> {
    let placeholder = "{{placeholder}}";
    match note_type {
        "task" => TaskTemplate {
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
        "session" => SessionTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
        }
        .render(),
        "goal" => GoalTemplate {
            id: placeholder,
            title: placeholder,
            slug: placeholder,
            context: placeholder,
            seq: "0",
            date: placeholder,
        }
        .render(),
        "research" => ResearchTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
            project: placeholder,
            slug: placeholder,
        }
        .render(),
        "concept" => ConceptTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
            project: placeholder,
            slug: placeholder,
        }
        .render(),
        "decision" => DecisionTemplate {
            id: placeholder,
            title: placeholder,
            date: placeholder,
            project: placeholder,
            slug: placeholder,
        }
        .render(),
        _ => {
            return Err(TemperError::Vault(format!(
                "No template found for '{note_type}'"
            )))
        }
    }
    .map_err(|e| TemperError::Vault(format!("Failed to render template: {e}")))
}

// ---------------------------------------------------------------------------
// Frontmatter utilities
// ---------------------------------------------------------------------------

/// Parse YAML frontmatter from markdown content
pub fn parse_frontmatter(content: &str) -> Option<serde_yaml::Value> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end = rest.find("---")?;
    let yaml_str = &rest[..end];
    serde_yaml::from_str(yaml_str).ok()
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

/// Replace a frontmatter field value in a markdown document
pub fn set_frontmatter_field(content: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut in_frontmatter = false;
    for line in &mut lines {
        if line.trim() == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter && line.starts_with(&format!("{key}:")) {
            *line = format!("{key}: {value}");
        }
    }
    lines.join("\n") + "\n"
}

/// Rename a frontmatter field key, preserving its value. Only operates within
/// the YAML frontmatter block — body content is not modified.
pub fn rename_frontmatter_field(content: &str, old_key: &str, new_key: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::with_capacity(lines.len());
    let mut in_frontmatter = false;
    let mut frontmatter_count = 0;

    for line in &lines {
        if line.trim() == "---" {
            frontmatter_count += 1;
            in_frontmatter = frontmatter_count == 1;
            result.push(line.to_string());
            continue;
        }
        if in_frontmatter && line.starts_with(&format!("{old_key}:")) {
            let rest = &line[old_key.len()..]; // includes the ':'
            result.push(format!("{new_key}{rest}"));
        } else {
            result.push(line.to_string());
        }
    }

    let joined = result.join("\n");
    if content.ends_with('\n') {
        joined + "\n"
    } else {
        joined
    }
}

/// Remove a field entirely from the frontmatter block. Body content is not modified.
pub fn remove_frontmatter_field(content: &str, key: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::with_capacity(lines.len());
    let mut in_frontmatter = false;
    let mut frontmatter_count = 0;

    for line in &lines {
        if line.trim() == "---" {
            frontmatter_count += 1;
            in_frontmatter = frontmatter_count == 1;
            result.push(line.to_string());
            continue;
        }
        if in_frontmatter && line.starts_with(&format!("{key}:")) {
            // skip this line — removes the field
            continue;
        }
        result.push(line.to_string());
    }

    let joined = result.join("\n");
    if content.ends_with('\n') {
        joined + "\n"
    } else {
        joined
    }
}

/// Insert a new field at the top of the frontmatter block (immediately after the
/// opening `---`). Body content is not modified.
pub fn insert_frontmatter_field(content: &str, key: &str, value: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::with_capacity(lines.len() + 1);
    let mut inserted = false;

    for line in &lines {
        result.push(line.to_string());
        // Insert after the first `---` opener
        if !inserted && line.trim() == "---" {
            result.push(format!("{key}: {value}"));
            inserted = true;
        }
    }

    let joined = result.join("\n");
    if content.ends_with('\n') {
        joined + "\n"
    } else {
        joined
    }
}

/// Write a note to the filesystem, creating parent directories as needed
pub fn write_note(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Replace the body of a markdown note, preserving YAML frontmatter.
pub fn replace_body(existing: &str, new_body: &str) -> String {
    let trimmed = existing.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---") {
        if let Some(end) = after_open.find("---") {
            let frontmatter_end = 3 + end + 3;
            let frontmatter = &trimmed[..frontmatter_end];
            return format!("{frontmatter}\n\n{new_body}");
        }
    }
    new_body.to_string()
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
