use askama::Template;
use temper_core::vault::Vault;

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
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context_name);
    let note_path = vault_layout.doc_file(&owner, context_name, "research", &slug);

    if note_path.exists() {
        if let Some(body) = stdin_content {
            let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&note_path)?;
            fm.set_body(body.to_string());
            fm.write_to(&note_path)?;

            crate::actions::runtime::publish_local_write_best_effort(
                &config.vault_root,
                &note_path,
            )?;

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
    let rendered = tmpl
        .render()
        .map_err(|e| crate::error::TemperError::Vault(format!("template error: {e}")))?;

    let mut fm = temper_core::frontmatter::Frontmatter::try_from(rendered.as_str())?;
    fm.set_managed_field(
        "temper-context",
        serde_json::Value::String(context_name.to_string()),
    );
    if let Some(body) = stdin_content {
        fm.set_body(body.to_string());
    }

    if let Some(parent) = note_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&note_path)?;

    crate::actions::runtime::publish_local_write_best_effort(&config.vault_root, &note_path)?;

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
    let event = Event::ResourceCreate {
        ts,
        doc_type: "research".to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        context: context_name.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}
