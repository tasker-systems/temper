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
    let filename = format!("{today} \u{2014} {slug}.md");
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
    let content = vault::set_frontmatter_field(&content, "context", &context_name);

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
    let event = Event::NoteCreate {
        ts,
        note_type: "session".to_string(),
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
        fm.get("id")
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
                task_content = vault::set_frontmatter_field(&task_content, "branch", &branch);
            }
        }
    }

    // Optionally update the task stage
    if let Some(s) = state {
        vault::validate_stage(s)?;
        task_content = vault::set_frontmatter_field(&task_content, "stage", s);
    }

    std::fs::write(&task_path, &task_content)?;
    Ok(())
}

/// List recent sessions, optionally filtered by context.
///
/// Scans `<context>/session/` dirs, parses frontmatter for date, sorts by date descending,
/// displays up to 20 entries.
pub fn list(config: &Config, context: Option<&str>, format: &str) -> Result<()> {
    let mut entries: Vec<SessionEntry> = Vec::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    for ctx in &contexts_to_scan {
        let session_dir = config.doc_type_dir(ctx, "session");
        if session_dir.is_dir() {
            collect_sessions(&session_dir, ctx, &mut entries)?;
        }
    }

    // Sort by date descending (most recent first)
    entries.sort_by(|a, b| b.date.cmp(&a.date));
    entries.truncate(20);

    if format == "json" {
        let json = serde_json::to_string_pretty(&entries).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    if entries.is_empty() {
        output::hint("No sessions found.");
        return Ok(());
    }

    output::plain(format!("{:<12} {:<20} Title", "Date", "Context"));
    output::dim("-".repeat(60));
    for entry in &entries {
        output::plain(format!(
            "{:<12} {:<20} {}",
            entry.date, entry.context, entry.title
        ));
    }

    Ok(())
}

#[derive(Serialize)]
struct SessionEntry {
    date: String,
    context: String,
    title: String,
}

fn collect_sessions(
    dir: &std::path::Path,
    context: &str,
    entries: &mut Vec<SessionEntry>,
) -> Result<()> {
    for file_entry in std::fs::read_dir(dir)? {
        let file_entry = file_entry?;
        let path = file_entry.path();
        if path.extension().is_some_and(|e| e == "md") {
            add_session_entry(&path, context, entries);
        }
    }
    Ok(())
}

fn add_session_entry(path: &std::path::Path, context: &str, entries: &mut Vec<SessionEntry>) {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Try to parse date from frontmatter first, then from filename prefix
    let date = parse_date_from_file(path)
        .or_else(|| extract_date_from_stem(&stem))
        .unwrap_or_else(|| "unknown".to_string());

    // Extract title: filename stem after the em-dash separator, or full stem
    let title = if let Some(pos) = stem.find(" \u{2014} ") {
        stem[pos + " \u{2014} ".len()..].to_string()
    } else {
        stem.clone()
    };

    entries.push(SessionEntry {
        date,
        context: context.to_string(),
        title,
    });
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
    let filename = format!("{today} \u{2014} {slug}.md");
    config.doc_type_dir(context, "session").join(filename)
}
