use std::path::PathBuf;

use askama::Template;
use chrono::Local;
use serde::Serialize;

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

    // Build path: <vault_root>/<context>/session/<date> — <slug>.md
    let slug = vault::slugify(note_title);
    let filename = format!("{today}-{slug}.md");
    let session_dir = config.doc_type_dir(&context_name, "session");
    let note_path = session_dir.join(&filename);

    if note_path.exists() {
        // File exists: replace body if stdin provided, otherwise no-op
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

    // File doesn't exist: create from session template
    let id = crate::ids::generate_id();
    let tmpl = SessionTemplate {
        id: &id,
        title: note_title,
        date: &today,
    };
    let content = tmpl
        .render()
        .map_err(|e| crate::error::TemperError::Vault(format!("template error: {e}")))?;

    // Set context field in frontmatter
    let content = vault::set_frontmatter_field(&content, "temper-context", &context_name);

    // If stdin content was piped, replace the template body
    let content = if let Some(body) = stdin_content {
        vault::replace_body(&content, body)
    } else {
        content
    };

    vault::write_note(&note_path, &content)?;

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
    let session_id = if let Some(fm) = vault::parse_frontmatter(&session_content) {
        fm.get("temper-id")
            .or_else(|| fm.get("temper-provisional-id"))
            .or_else(|| fm.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Read the task file
    let task_path = config
        .doc_type_dir(&task_info.context, "task")
        .join(format!("{}.md", task_info.slug));
    let mut task_content = std::fs::read_to_string(&task_path)?;

    // Add/append to the sessions list in frontmatter
    if !session_id.is_empty() {
        if task_content.contains("\nsessions:") {
            // sessions key already exists — append to the list
            let sessions_marker = "\nsessions:";
            if let Some(pos) = task_content.find(sessions_marker) {
                let after_marker = pos + sessions_marker.len();
                let insert_pos = after_marker;
                let new_entry = format!("\n  - {session_id}");
                task_content.insert_str(insert_pos, &new_entry);
            }
        } else {
            // Insert sessions field before the closing --- of frontmatter
            let trimmed_start = if task_content.starts_with("---") {
                3
            } else {
                0
            };
            if let Some(close_pos) = task_content[trimmed_start..].find("\n---") {
                let insert_at = trimmed_start + close_pos;
                let new_field = format!("\nsessions:\n  - {session_id}");
                task_content.insert_str(insert_at, &new_field);
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
                task_content =
                    vault::set_frontmatter_field(&task_content, "temper-branch", &branch);
            }
        }
    }

    // Optionally update the task stage
    if let Some(s) = state {
        vault::validate_stage(s)?;
        task_content = vault::set_frontmatter_field(&task_content, "temper-stage", s);
    }

    std::fs::write(&task_path, &task_content)?;
    Ok(())
}

/// Show a single session's raw markdown content.
///
/// Searches `<context>/session/` dirs for a file whose slug matches the given
/// `slug_or_suffix`. Matches against the title portion of the filename (after the
/// em-dash separator) or the full stem (date + title).
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    let needle = vault::slugify(slug_or_suffix);
    let mut matches: Vec<(SessionEntry, PathBuf)> = Vec::new();

    for ctx in &contexts_to_scan {
        let session_dir = config.doc_type_dir(ctx, "session");
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
                // Legacy format: "2026-04-05 — slug"
                stem[pos + " \u{2014} ".len()..].to_string()
            } else if stem.len() > 10 && stem.as_bytes().get(10) == Some(&b'-') {
                // New format: "2026-04-05-slug"
                stem[11..].to_string()
            } else {
                stem.clone()
            };

            // Match: exact title slug, or full stem contains the needle
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

    // Sort by date descending, take the most recent match
    matches.sort_by(|a, b| b.0.date.cmp(&a.0.date));
    let (entry, path) = &matches[0];

    if format == "json" {
        #[derive(Serialize)]
        struct SessionShow<'a> {
            date: &'a str,
            context: &'a str,
            title: &'a str,
            path: String,
            content: String,
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::error::TemperError::Vault(e.to_string()))?;
        let relative = path.strip_prefix(&config.vault_root).unwrap_or(path);
        let info = SessionShow {
            date: &entry.date,
            context: &entry.context,
            title: &entry.title,
            path: relative.to_string_lossy().to_string(),
            content,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| crate::error::TemperError::Vault(e.to_string()))?;
    print!("{content}");
    Ok(())
}

#[derive(Serialize)]
struct SessionEntry {
    date: String,
    context: String,
    title: String,
}

fn parse_date_from_file(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let fm = vault::parse_frontmatter(&content)?;
    let date = fm.get("date")?;
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
    let slug = vault::slugify(title);
    let filename = format!("{today}-{slug}.md");
    config.doc_type_dir(context, "session").join(filename)
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
            skill_output: PathBuf::from("/tmp/test-skill"),
        };
        (tmp, config)
    }

    fn write_session(config: &Config, context: &str, date: &str, slug: &str, body: &str) {
        let dir = config.doc_type_dir(context, "session");
        let filename = format!("{date}-{slug}.md");
        let content = format!(
            "---\ntemper-id: \"test-id\"\ntemper-type: session\ndate: {date}\n---\n\n{body}"
        );
        fs::write(dir.join(filename), content).unwrap();
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
