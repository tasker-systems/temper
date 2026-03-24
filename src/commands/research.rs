use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::project;
use crate::vault;
use chrono::Local;

pub fn save(
    config: &Config,
    title: &str,
    project: Option<&str>,
    stdin_content: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    let project_name: String = if let Some(p) = project {
        p.to_string()
    } else if let Ok(cwd) = std::env::current_dir() {
        project::resolve_from_cwd(&cwd, &config.projects)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "general".to_string())
    } else {
        "general".to_string()
    };

    let slug = format!("{today}-{}", vault::slugify(title));
    let filename = format!("{today} \u{2014} {title}.md");
    let research_dir = config.vault_root.join("research").join(&project_name);
    let note_path = research_dir.join(&filename);

    if note_path.exists() {
        if let Some(body) = stdin_content {
            let existing = std::fs::read_to_string(&note_path)?;
            let updated = replace_body(&existing, body);
            std::fs::write(&note_path, updated)?;
            let relative = note_path
                .strip_prefix(&config.vault_root)
                .unwrap_or(&note_path);
            output::success(format!("Updated: {}", relative.display()));
        }
        return Ok(());
    }

    let id = crate::ids::generate_id();
    let templates_rel = config
        .templates_dir
        .strip_prefix(&config.vault_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "templates".to_string());

    let mut content = vault::render_template_with_vars(
        &config.vault_root,
        &templates_rel,
        "research",
        title,
        &[("project", &project_name), ("id", &id), ("slug", &slug)],
    )?;

    content = vault::set_frontmatter_field(&content, "project", &project_name);

    if let Some(body) = stdin_content {
        content = replace_body(&content, body);
    }

    vault::write_note(&note_path, &content)?;

    let relative = note_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();

    if format == "json" {
        let json = serde_json::json!({
            "title": title,
            "project": project_name,
            "path": relative_str,
            "date": today,
            "id": id,
            "slug": slug,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
    } else {
        output::success(format!("Created: {relative_str}"));
    }

    let ts = Local::now().to_rfc3339();
    let event = Event::NoteCreate {
        ts,
        note_type: "research".to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        project: project_name,
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}

fn replace_body(existing: &str, new_body: &str) -> String {
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
