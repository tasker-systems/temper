use crate::error::{Result, TemperError};
use chrono::Local;
use std::path::{Path, PathBuf};

const EMBEDDED_SESSION: &str = include_str!("templates/session.md");
const EMBEDDED_TICKET: &str = include_str!("templates/ticket.md");
const EMBEDDED_MILESTONE: &str = include_str!("templates/milestone.md");

fn embedded_template(note_type: &str) -> Option<&'static str> {
    match note_type {
        "session" => Some(EMBEDDED_SESSION),
        "ticket" => Some(EMBEDDED_TICKET),
        "milestone" => Some(EMBEDDED_MILESTONE),
        _ => None,
    }
}

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

/// Write a note to the filesystem, creating parent directories as needed
pub fn write_note(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Read a note template, fill in {{date}} and {{title}}.
/// Looks for a template file in `vault_root/templates_dir/{note_type}.md` first,
/// then falls back to the embedded template.
pub fn render_template(
    vault_root: &Path,
    templates_dir: &str,
    note_type: &str,
    title: &str,
) -> Result<String> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let template_path = vault_root
        .join(templates_dir)
        .join(format!("{note_type}.md"));

    let content = if template_path.exists() {
        std::fs::read_to_string(&template_path).map_err(|e| {
            TemperError::Vault(format!(
                "Failed to read template {}: {e}",
                template_path.display()
            ))
        })?
    } else if let Some(embedded) = embedded_template(note_type) {
        embedded.to_string()
    } else {
        return Err(TemperError::Vault(format!(
            "No template found for note type '{note_type}'"
        )));
    };

    Ok(content
        .replace("{{date}}", &today)
        .replace("{{title}}", title))
}

/// Render a template with the standard {{date}} and {{title}} substitutions
/// plus additional custom variables.
pub fn render_template_with_vars(
    vault_root: &Path,
    templates_dir: &str,
    note_type: &str,
    title: &str,
    vars: &[(&str, &str)],
) -> Result<String> {
    let mut content = render_template(vault_root, templates_dir, note_type, title)?;
    for (key, value) in vars {
        content = content.replace(&format!("{{{{{key}}}}}"), value);
    }
    Ok(content)
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
