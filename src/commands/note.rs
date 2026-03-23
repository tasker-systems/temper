use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::{Result, TemperError};
use crate::vault;
use chrono::Local;

/// Create a new note from a template.
///
/// - `note_type`: any template name in the templates dir (e.g. "session", "concept")
/// - `title`: note title (used in filename and template substitution)
/// - `project`: optional project tag written into frontmatter
/// - `from_stdin`: if true, read body content from stdin and merge with template
pub fn create(
    config: &Config,
    note_type: &str,
    title: &str,
    project: Option<&str>,
    from_stdin: bool,
) -> Result<()> {
    let templates_rel = config
        .templates_dir
        .strip_prefix(&config.vault_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "templates".to_string());

    let mut content = vault::render_template(
        &config.vault_root,
        &templates_rel,
        note_type,
        title,
    )?;

    // Determine output path: <vault_root>/<note_type>s/<title>.md
    let slug = vault::slugify(title);
    let note_dir = config.vault_root.join(format!("{note_type}s"));
    let note_path = note_dir.join(format!("{slug}.md"));

    if note_path.exists() {
        return Err(TemperError::Vault(format!(
            "Note already exists: {}",
            note_path.display()
        )));
    }

    // Apply --project to frontmatter if provided
    if let Some(proj) = project {
        content = vault::set_frontmatter_field(&content, "project", proj);
    }

    // If --stdin, read body from stdin and merge (keep frontmatter, replace body)
    if from_stdin {
        let stdin_content = read_stdin()?;
        if !stdin_content.is_empty() {
            content = merge_stdin_with_template(&content, &stdin_content);
        }
    }

    vault::write_note(&note_path, &content)?;

    let relative = note_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();
    println!("Created: {relative_str}");

    let ts = Local::now().to_rfc3339();
    let event = Event::NoteCreate {
        ts,
        note_type: note_type.to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        project: project.unwrap_or("").to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}

fn read_stdin() -> Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn merge_stdin_with_template(template: &str, stdin_content: &str) -> String {
    let trimmed = template.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---") {
        if let Some(end) = after_open.find("---") {
            let frontmatter_end = 3 + end + 3;
            let frontmatter = &trimmed[..frontmatter_end];
            return format!("{frontmatter}\n\n{stdin_content}");
        }
    }
    stdin_content.to_string()
}
