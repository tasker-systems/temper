use chrono::Local;
use temper_core::vault::Vault;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::vault;
use crate::vault_backend::per_doctype::{self, DoctypeFields, WriteArgs};

/// Create or update today's research note.
///
/// Two paths:
/// - If a research file with this slug already exists on disk, this is the
///   save-or-update overload: when `stdin_content` is `Some(_)`, the body is
///   replaced in place; otherwise the call is a no-op. This branch stays at
///   the surface because `per_doctype::write_research` hard-errors on existing
///   slugs.
/// - If the file does not exist, delegate the bare file-write (template render,
///   managed-meta overlay, body application, write) to
///   `per_doctype::write_research`. The wrapper retains publish-as-tail-action,
///   discovery emission, and output.
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
        // File exists: replace body if stdin provided, otherwise no-op.
        // This is the save-or-update overload — preserved at the surface layer
        // because `per_doctype::write_research` hard-errors on existing slugs.
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

    // File doesn't exist: delegate the bare file-write (template render,
    // managed-meta overlay, body application, write) to per_doctype::write_research.
    // The wrapper retains publish-as-tail-action, discovery emission, and output.
    let body = stdin_content.unwrap_or("");
    let result = per_doctype::write_for(WriteArgs {
        doctype: "research",
        title,
        slug: &slug,
        context: context_name,
        body,
        open_meta: None,
        vault_root: &config.vault_root,
        owner: &owner,
        config,
        doctype_fields: Some(DoctypeFields::Research),
    })?;

    crate::actions::runtime::publish_local_write_best_effort(&config.vault_root, &result.abs_path)?;

    let relative_str = result.rel_path.clone();
    let id = result.resource_id.to_string();

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
