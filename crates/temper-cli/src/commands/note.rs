use serde::Serialize;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;
use chrono::Local;

/// Create a new note from a template.
///
/// - `note_type`: any template name (e.g. "session", "concept")
/// - `title`: note title (used in filename and template substitution)
/// - `project`: optional project tag written into frontmatter
pub fn create(
    config: &Config,
    note_type: &str,
    title: &str,
    project: Option<&str>,
    format: &str,
) -> Result<()> {
    // Render the template using askama via get_template with actual values
    let mut content = vault::get_template(note_type)?;

    // Replace placeholders with actual values
    let today = Local::now().format("%Y-%m-%d").to_string();
    let id = crate::ids::generate_id();
    content = content.replace("{{placeholder}}", title);
    content = vault::set_frontmatter_field(&content, "title", title);
    content = vault::set_frontmatter_field(&content, "date", &today);
    content = vault::set_frontmatter_field(&content, "temper-id", &format!("\"{id}\""));

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

    // If stdin is piped, read body from stdin and merge (keep frontmatter, replace body)
    if let Some(stdin_content) = vault::read_stdin_if_piped() {
        content = vault::replace_body(&content, &stdin_content);
    }

    vault::write_note(&note_path, &content)?;

    let relative = note_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();

    if format == "json" {
        #[derive(Serialize)]
        struct NoteCreated<'a> {
            note_type: &'a str,
            title: &'a str,
            path: &'a str,
            project: Option<&'a str>,
        }
        let info = NoteCreated {
            note_type,
            title,
            path: &relative_str,
            project,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
    } else {
        output::success(format!("Created: {relative_str}"));
    }

    let ts = Local::now().to_rfc3339();
    let event = Event::NoteCreate {
        ts,
        note_type: note_type.to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        context: project.unwrap_or("").to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}
