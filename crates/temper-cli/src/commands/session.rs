use std::path::PathBuf;

use askama::Template;
use chrono::Local;
use serde::Serialize;
use temper_core::vault::Vault;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::templates::SessionTemplate;
use crate::vault;

/// Create or update today's session note.
///
/// Path: `<vault_root>/<context>/session/<date> — <slug>.md`
///
/// - If `context` is None and `task` is provided, infers context from the task
/// - If `context` is None and no task, falls back to "general"
/// - `title` defaults to today's date if omitted
/// - The filename uses a slugified version of the title
/// - If the file already exists and `stdin_content` is None: no-op (idempotent)
/// - If the file already exists and `stdin_content` is Some: replace body, preserve frontmatter
/// - If `task` is provided, links the session to the task (updates sessions list in task frontmatter)
/// - If `state` is also provided, updates the task's stage field
pub fn save(
    config: &Config,
    title: Option<&str>,
    context: Option<&str>,
    stdin_content: Option<&str>,
    task: Option<&str>,
    state: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    // Infer context from task if not explicitly provided
    let inferred_context = if context.is_none() {
        task.and_then(|slug| {
            crate::actions::task::find_task(config, slug, None)
                .ok()
                .flatten()
                .map(|info| info.context)
        })
    } else {
        None
    };
    let context_name = context
        .map(String::from)
        .or(inferred_context)
        .unwrap_or_else(|| "general".to_string());

    let note_title = title.unwrap_or(&today);

    // Build path: <vault_root>/<owner>/<context>/session/<date>-<slug>.md
    let title_slug = vault::slugify(note_title);
    let slug = format!("{today}-{title_slug}");
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(&context_name);
    let note_path = vault_layout.doc_file(&owner, &context_name, "session", &slug);

    if note_path.exists() {
        // File exists: replace body if stdin provided, otherwise no-op
        if let Some(body) = stdin_content {
            let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&note_path)?;
            fm.set_body(body.to_string());
            fm.write_to(&note_path)?;
            let relative = note_path
                .strip_prefix(&config.vault_root)
                .unwrap_or(&note_path);
            output::success(format!("Updated: {}", relative.display()));
        }
        return Ok(());
    }

    // File doesn't exist: create from session template
    let id = crate::ids::generate_id();
    let tmpl = SessionTemplate {
        id: &id,
        title: note_title,
        date: &today,
    };
    let rendered = tmpl
        .render()
        .map_err(|e| crate::error::TemperError::Vault(format!("template error: {e}")))?;

    let mut fm = temper_core::frontmatter::Frontmatter::try_from(rendered.as_str())?;
    fm.set_managed_field(
        "temper-context",
        serde_json::Value::String(context_name.clone()),
    );
    if let Some(body) = stdin_content {
        fm.set_body(body.to_string());
    }

    if let Some(parent) = note_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&note_path)?;

    let relative = note_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&note_path);
    let relative_str = relative.to_string_lossy();

    let ts = Local::now().to_rfc3339();

    if format == "json" {
        #[derive(Serialize)]
        struct SessionCreated<'a> {
            title: &'a str,
            context: &'a str,
            path: &'a str,
            date: &'a str,
        }
        let info = SessionCreated {
            title: note_title,
            context: &context_name,
            path: &relative_str,
            date: &today,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
    } else {
        output::success(format!("Created: {relative_str}"));
    }
    let event = Event::ResourceCreate {
        ts,
        doc_type: "session".to_string(),
        title: note_title.to_string(),
        path: relative_str.to_string(),
        context: context_name.clone(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    // Link session to task if provided
    if let Some(task_slug) = task {
        link_session_to_task(config, &note_path, task_slug, state)?;
    }

    Ok(())
}

/// Link a session note to a task: update the task's sessions list and optionally its stage.
fn link_session_to_task(
    config: &Config,
    session_path: &std::path::Path,
    task_slug: &str,
    state: Option<&str>,
) -> Result<()> {
    // Find the task
    let task_info = crate::commands::task::find_task(config, task_slug, None)?
        .ok_or_else(|| crate::error::TemperError::Vault(format!("task not found: {task_slug}")))?;

    // Extract the session's id from its frontmatter
    let session_content = std::fs::read_to_string(session_path)?;
    let session_id = temper_core::frontmatter::Frontmatter::try_from(session_content.as_str())
        .ok()
        .and_then(|fm| {
            fm.value()
                .get("temper-id")
                .or_else(|| fm.value().get("temper-provisional-id"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .unwrap_or_default();

    // Read the task file
    let task_vault = temper_core::vault::Vault::new(&config.vault_root);
    let task_owner = config.owner_for_context(&task_info.context);
    let task_path = task_vault.doc_file(&task_owner, &task_info.context, "task", &task_info.slug);
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&task_path)?;

    // Append session_id to the sessions list (or create it fresh).
    if !session_id.is_empty() {
        let mapping = fm
            .value_mut()
            .as_mapping_mut()
            .expect("Frontmatter invariant: value is a mapping");
        let sessions_key = serde_yaml::Value::String("sessions".to_string());
        let new_entry = serde_yaml::Value::String(session_id.clone());
        match mapping.get_mut(&sessions_key) {
            Some(serde_yaml::Value::Sequence(seq)) => seq.push(new_entry),
            _ => {
                mapping.insert(sessions_key, serde_yaml::Value::Sequence(vec![new_entry]));
            }
        }
    }

    // Optionally update the git branch field
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
    {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() && branch != "HEAD" {
                fm.set_managed_field("temper-branch", serde_json::Value::String(branch));
            }
        }
    }

    // Optionally update the task stage
    if let Some(s) = state {
        vault::validate_stage(s)?;
        fm.set_managed_field("temper-stage", serde_json::Value::String(s.to_string()));
    }

    fm.write_to(&task_path)?;
    Ok(())
}

/// Show a single session's content.
///
/// Local mode: scans the vault for a matching session file, then uses the
/// three-tier freshness ladder to produce content. JSON output preserves the
/// `SessionShow` struct (date, context, title, path, content).
///
/// Cloud mode: requires a context; resolves the session id via
/// `GET /api/resources/by-uri` and fetches content via
/// `GET /api/resources/{id}/content`. No disk writes. JSON emits a
/// `SessionShow`-shaped struct with an empty path field.
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::{runtime, show_cache};
    use std::time::Duration;
    use temper_core::types::VaultState;

    #[derive(Serialize)]
    struct SessionShow {
        date: String,
        context: String,
        title: String,
        path: String,
        content: String,
    }

    let vault_state = VaultState::from_env();

    match vault_state {
        VaultState::Local => {
            let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
                vec![ctx.to_string()]
            } else {
                config.contexts.clone()
            };

            let needle = vault::slugify(slug_or_suffix);
            let mut matches: Vec<(SessionEntry, PathBuf)> = Vec::new();
            let vault_layout = Vault::new(&config.vault_root);

            for ctx in &contexts_to_scan {
                let owner = config.owner_for_context(ctx);
                let session_dir = vault_layout.doc_type_dir(&owner, ctx, "session");
                if !session_dir.is_dir() {
                    continue;
                }
                for file_entry in std::fs::read_dir(&session_dir)? {
                    let file_entry = file_entry?;
                    let path = file_entry.path();
                    if path.extension().is_none_or(|e| e != "md") {
                        continue;
                    }
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let title_slug = if let Some(pos) = stem.find(" \u{2014} ") {
                        stem[pos + " \u{2014} ".len()..].to_string()
                    } else if stem.len() > 10 && stem.as_bytes().get(10) == Some(&b'-') {
                        stem[11..].to_string()
                    } else {
                        stem.clone()
                    };

                    if title_slug == needle
                        || vault::slugify(&stem) == needle
                        || title_slug.contains(&needle)
                    {
                        let date = parse_date_from_file(&path)
                            .or_else(|| extract_date_from_stem(&stem))
                            .unwrap_or_else(|| "unknown".to_string());
                        matches.push((
                            SessionEntry {
                                date,
                                context: ctx.clone(),
                                title: title_slug,
                            },
                            path,
                        ));
                    }
                }
            }

            if matches.is_empty() {
                return Err(crate::error::TemperError::Vault(format!(
                    "session not found: {slug_or_suffix}"
                )));
            }

            matches.sort_by(|a, b| b.0.date.cmp(&a.0.date));
            let (entry, path) = matches.remove(0);

            // Tier 0: serve from disk if fresh — no runtime or API needed.
            if let Some(content) = show_cache::read_if_fresh(
                &path,
                std::time::Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
            )? {
                if format == "json" {
                    let relative = path.strip_prefix(&config.vault_root).unwrap_or(&path);
                    let info = SessionShow {
                        date: entry.date,
                        context: entry.context,
                        title: entry.title,
                        path: relative.to_string_lossy().to_string(),
                        content,
                    };
                    let json = serde_json::to_string_pretty(&info).unwrap_or_default();
                    println!("{json}");
                    return Ok(());
                }
                print!("{content}");
                return Ok(());
            }

            let entry_ctx = entry.context.clone();
            let entry_title = entry.title.clone();
            let config_clone = config.clone();

            let (content, resolved_path) = runtime::with_client(|client| {
                Box::pin(async move {
                    let id = super::resource::resolve_resource_id(
                        &config_clone,
                        client,
                        "session",
                        &entry_title,
                        Some(&entry_ctx),
                        VaultState::Local,
                    )
                    .await?;
                    let result = show_cache::fetch(show_cache::ShowCacheParams {
                        client,
                        resource_id: id,
                        local_path: &path,
                        debounce: Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
                    })
                    .await?;
                    Ok((result.content, path))
                })
            })?;

            if format == "json" {
                let relative = resolved_path
                    .strip_prefix(&config.vault_root)
                    .unwrap_or(&resolved_path);
                let info = SessionShow {
                    date: entry.date,
                    context: entry.context,
                    title: entry.title,
                    path: relative.to_string_lossy().to_string(),
                    content,
                };
                let json = serde_json::to_string_pretty(&info).unwrap_or_default();
                println!("{json}");
                return Ok(());
            }

            print!("{content}");
            Ok(())
        }
        VaultState::Cloud => {
            let ctx_s = context.map(str::to_string);
            let slug_s = slug_or_suffix.to_string();
            let config_clone = config.clone();

            let body = runtime::with_client(|client| {
                Box::pin(async move {
                    let id = super::resource::resolve_resource_id(
                        &config_clone,
                        client,
                        "session",
                        &slug_s,
                        ctx_s.as_deref(),
                        VaultState::Cloud,
                    )
                    .await?;
                    let resp = client
                        .resources()
                        .content(*id.as_uuid())
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;
                    Ok(resp.markdown)
                })
            })?;

            if format == "json" {
                let ctx = context.unwrap_or("");
                let info = SessionShow {
                    date: String::new(),
                    context: ctx.to_string(),
                    title: slug_or_suffix.to_string(),
                    path: String::new(),
                    content: body,
                };
                let json = serde_json::to_string_pretty(&info).unwrap_or_default();
                println!("{json}");
                return Ok(());
            }

            print!("{body}");
            Ok(())
        }
    }
}

#[derive(Serialize)]
struct SessionEntry {
    date: String,
    context: String,
    title: String,
}

fn parse_date_from_file(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let fm = temper_core::frontmatter::Frontmatter::try_from(content.as_str()).ok()?;
    let date = fm.value().get("date")?;
    Some(date.as_str()?.to_string())
}

fn extract_date_from_stem(stem: &str) -> Option<String> {
    // Expect stem to start with YYYY-MM-DD
    if stem.len() >= 10 {
        let candidate = &stem[..10];
        if candidate.len() == 10
            && candidate.chars().nth(4) == Some('-')
            && candidate.chars().nth(7) == Some('-')
        {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Return path that would be used for a session note (for testing/preview).
#[allow(dead_code)]
pub fn session_path(config: &Config, context: &str, title: &str) -> PathBuf {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let title_slug = vault::slugify(title);
    let slug = format!("{today}-{title_slug}");
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    vault_layout.doc_file(&owner, context, "session", &slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn test_vault() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().to_path_buf();
        let state_dir = vault_root.join(".temper");
        fs::create_dir_all(&state_dir).unwrap();
        fs::create_dir_all(vault_root.join("temper/session")).unwrap();
        fs::create_dir_all(vault_root.join("default/session")).unwrap();
        let config = Config {
            vault_root,
            state_dir,
            contexts: vec!["temper".to_string(), "default".to_string()],
            subscriptions: Vec::new(),
            skill_output: PathBuf::from("/tmp/test-skill"),
        };
        (tmp, config)
    }

    fn write_session(config: &Config, context: &str, date: &str, slug: &str, body: &str) {
        let vault_layout = Vault::new(&config.vault_root);
        let owner = config.owner_for_context(context);
        let full_slug = format!("{date}-{slug}");
        let path = vault_layout.doc_file(&owner, context, "session", &full_slug);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = format!(
            "---\ntemper-id: \"test-id\"\ntemper-type: session\ndate: {date}\n---\n\n{body}"
        );
        fs::write(path, content).unwrap();
    }

    #[test]
    fn show_exact_slug_match() {
        let (_tmp, config) = test_vault();
        write_session(
            &config,
            "temper",
            "2026-04-04",
            "my-session",
            "## Goal\nTest",
        );
        let result = show(&config, "my-session", Some("temper"), "text");
        assert!(result.is_ok());
    }

    #[test]
    fn show_partial_slug_match() {
        let (_tmp, config) = test_vault();
        write_session(
            &config,
            "temper",
            "2026-04-04",
            "fix-temper-init-data-hygiene",
            "## Goal\nFix stuff",
        );
        let result = show(&config, "fix-temper-init", Some("temper"), "text");
        assert!(result.is_ok());
    }

    #[test]
    fn show_not_found_returns_error() {
        let (_tmp, config) = test_vault();
        write_session(&config, "temper", "2026-04-04", "some-session", "body");
        let result = show(&config, "nonexistent", Some("temper"), "text");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("session not found"), "got: {err}");
    }

    #[test]
    fn show_returns_most_recent_when_multiple_match() {
        let (_tmp, config) = test_vault();
        write_session(&config, "temper", "2026-04-01", "my-work", "## Goal\nOlder");
        write_session(&config, "temper", "2026-04-04", "my-work", "## Goal\nNewer");
        // Both match "my-work" — should get the 04-04 one
        let result = show(&config, "my-work", Some("temper"), "json");
        assert!(result.is_ok());
    }

    #[test]
    fn show_scans_all_contexts_when_none_specified() {
        let (_tmp, config) = test_vault();
        write_session(
            &config,
            "default",
            "2026-04-04",
            "cross-context",
            "## Goal\nHere",
        );
        let result = show(&config, "cross-context", None, "text");
        assert!(result.is_ok());
    }

    #[test]
    fn show_wrong_context_returns_error() {
        let (_tmp, config) = test_vault();
        write_session(&config, "temper", "2026-04-04", "only-in-temper", "body");
        let result = show(&config, "only-in-temper", Some("default"), "text");
        assert!(result.is_err());
    }
}
