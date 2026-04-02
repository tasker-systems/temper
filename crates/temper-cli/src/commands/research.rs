use askama::Template;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::templates::ResearchTemplate;
use crate::vault;
use chrono::Local;

pub fn save(
    config: &Config,
    title: &str,
    context: Option<&str>,
    stdin_content: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    let context_name = context.unwrap_or("general");

    let slug = format!("{today}-{}", vault::slugify(title));
    let filename = format!("{today} \u{2014} {title}.md");
    let research_dir = config.vault_root.join("research").join(context_name);
    let note_path = research_dir.join(&filename);

    if note_path.exists() {
        if let Some(body) = stdin_content {
            let existing = std::fs::read_to_string(&note_path)?;
            let updated = vault::replace_body(&existing, body);
            std::fs::write(&note_path, updated)?;
            let relative = note_path
                .strip_prefix(&config.vault_root)
                .unwrap_or(&note_path);
            output::success(format!("Updated: {}", relative.display()));
        }
        return Ok(());
    }

    let id = crate::ids::generate_id();
    let tmpl = ResearchTemplate {
        id: &id,
        title,
        date: &today,
        project: context_name,
        slug: &slug,
    };
    let mut content = tmpl
        .render()
        .map_err(|e| crate::error::TemperError::Vault(format!("template error: {e}")))?;

    content = vault::set_frontmatter_field(&content, "project", context_name);

    if let Some(body) = stdin_content {
        content = vault::replace_body(&content, body);
    }

    vault::write_note(&note_path, &content)?;

    let relative = note_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();

    if format == "json" {
        let json = serde_json::json!({
            "title": title,
            "project": context_name,
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
        project: context_name.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}
