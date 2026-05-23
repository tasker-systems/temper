use askama::Template;
use chrono::Local;
use std::path::PathBuf;
use temper_core::error::TemperError;
use temper_core::frontmatter::Frontmatter;
use temper_core::types::ids::ResourceId;
use temper_core::vault::Vault;
use uuid::Uuid;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::templates::ResearchTemplate;
use crate::vault;

#[derive(Debug)]
struct InlineWriteResult {
    resource_id: Uuid,
    abs_path: PathBuf,
    rel_path: String,
}

/// Write a new research note to the vault using the `ResearchTemplate`.
///
/// Hard-errors if the target slug already exists on disk. The surface-side
/// save-or-update overload in `save` handles the existing-file case before
/// calling here.
///
/// Does not publish, emit discovery events, or print output — those are the
/// caller's (`save`'s) responsibility.
fn write_research_inline(
    config: &Config,
    title: &str,
    context: &str,
    slug: &str,
    body: &str,
) -> temper_core::error::Result<InlineWriteResult> {
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let abs_path = vault_layout.doc_file(&owner, context, "research", slug);
    let rel_path = vault_layout.rel_path(&owner, context, "research", slug);

    if abs_path.exists() {
        return Err(TemperError::Vault(format!(
            "research already exists: {slug}"
        )));
    }

    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TemperError::Vault(e.to_string()))?;
    }

    let id_str = crate::ids::generate_id();
    let date = Local::now().format("%Y-%m-%d").to_string();

    let tmpl = ResearchTemplate {
        id: &id_str,
        title,
        date: &date,
        project: context,
        slug,
    };
    let rendered = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;

    let mut fm = Frontmatter::try_from(rendered.as_str())?;
    let meta = temper_core::types::ManagedMeta {
        doc_type: Some("research".to_string()),
        context: Some(context.to_string()),
        title: Some(title.to_string()),
        ..Default::default()
    };
    fm.set_managed_meta(&meta);

    if !body.is_empty() {
        fm.set_body(body.to_string());
    }

    fm.write_to(&abs_path)?;

    let resource_id = Uuid::parse_str(&id_str)
        .map(ResourceId::from)
        .map_err(|e| {
            TemperError::Vault(format!("generated id is not a valid UUID: {id_str}: {e}"))
        })?;

    Ok(InlineWriteResult {
        resource_id: Uuid::from(resource_id),
        abs_path,
        rel_path,
    })
}

/// Create or update today's research note.
///
/// Two paths:
/// - If a research file with this slug already exists on disk, this is the
///   save-or-update overload: when `stdin_content` is `Some(_)`, the body is
///   replaced in place; otherwise the call is a no-op. This branch stays at
///   the surface because `write_research_inline` hard-errors on existing slugs.
/// - If the file does not exist, delegate the bare file-write (template render,
///   managed-meta overlay, body application, write) to `write_research_inline`.
///   The wrapper retains publish-as-tail-action, discovery emission, and output.
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
        // because `write_research_inline` hard-errors on existing slugs.
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
    // managed-meta overlay, body application, write) to write_research_inline.
    // The wrapper retains publish-as-tail-action, discovery emission, and output.
    let body = stdin_content.unwrap_or("");
    let result = write_research_inline(config, title, context_name, &slug, body)?;

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

#[cfg(test)]
mod inline_research_write_tests {
    use std::path::Path;

    use super::*;
    use tempfile::TempDir;

    fn make_config(vault_root: &Path) -> Config {
        Config {
            vault_root: vault_root.to_path_buf(),
            state_dir: vault_root.join(".temper"),
            contexts: vec!["temper".to_string()],
            subscriptions: vec![],
            skill_output: vault_root.join("skills"),
            profile_slug: None,
        }
    }

    #[test]
    fn inline_write_research_creates_file_with_correct_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path());
        let result = write_research_inline(
            &config,
            "Sample Title",
            "temper",
            "2026-05-23-sample-title",
            "body text",
        )
        .expect("write must succeed");

        assert!(result.abs_path.exists(), "file must exist");
        let fm = Frontmatter::parse_file(&result.abs_path).expect("must parse");
        let mapping = fm
            .value()
            .as_mapping()
            .expect("frontmatter must be mapping");
        let get = |key: &str| {
            mapping
                .get(serde_yaml::Value::String(key.to_string()))
                .cloned()
        };
        assert_eq!(
            get("temper-title"),
            Some(serde_yaml::Value::String("Sample Title".to_string())),
            "temper-title must be 'Sample Title'"
        );
        assert_eq!(
            get("temper-slug"),
            Some(serde_yaml::Value::String(
                "2026-05-23-sample-title".to_string()
            )),
            "temper-slug must be '2026-05-23-sample-title'"
        );
        assert_eq!(fm.body().trim(), "body text", "body must be 'body text'");
    }

    #[test]
    fn inline_write_research_errors_on_existing_slug() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(tmp.path());
        write_research_inline(&config, "T", "temper", "2026-05-23-t", "").expect("first write ok");
        let err = write_research_inline(&config, "T", "temper", "2026-05-23-t", "")
            .expect_err("second write must error");
        assert!(
            matches!(err, TemperError::Vault(ref m) if m.contains("already exists")),
            "expected Vault(already exists) error; got {err:?}"
        );
    }
}
