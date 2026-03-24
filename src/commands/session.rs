use std::path::PathBuf;

use chrono::Local;
use serde::Serialize;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::Result;
use crate::output;
use crate::project;
use crate::vault;

/// Create or update today's session note.
///
/// Path: `<vault_root>/<sessions_dir>/<project>/<date> — <title>.md`
///
/// - If `project` is None, infers from CWD; falls back to "general"
/// - `title` defaults to today's date if omitted
/// - If the file already exists and `stdin_content` is None: no-op (idempotent)
/// - If the file already exists and `stdin_content` is Some: replace body, preserve frontmatter
/// - If `ticket` is provided, links the session to the ticket (updates sessions list in ticket frontmatter)
/// - If `state` is also provided, updates the ticket's stage field
pub fn save(
    config: &Config,
    title: Option<&str>,
    project: Option<&str>,
    stdin_content: Option<&str>,
    ticket: Option<&str>,
    state: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();

    // Resolve project
    let project_name: String = if let Some(p) = project {
        p.to_string()
    } else if let Ok(cwd) = std::env::current_dir() {
        project::resolve_from_cwd(&cwd, &config.projects)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "general".to_string())
    } else {
        "general".to_string()
    };

    let note_title = title.unwrap_or(&today);

    // Build path: <sessions_dir>/<project>/<date> — <title>.md
    let filename = format!("{today} \u{2014} {note_title}.md");
    let session_project_dir = config.sessions_dir.join(&project_name);
    let note_path = session_project_dir.join(&filename);

    if note_path.exists() {
        // File exists: replace body if stdin provided, otherwise no-op
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

    // File doesn't exist: create from session template
    let templates_rel = config
        .templates_dir
        .strip_prefix(&config.vault_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "templates".to_string());

    let id = crate::ids::generate_id();
    let content = vault::render_template_with_vars(
        &config.vault_root,
        &templates_rel,
        "session",
        note_title,
        &[("project", &project_name), ("id", &id)],
    )?;

    // Set project field in frontmatter
    let content = vault::set_frontmatter_field(&content, "project", &project_name);

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
            project: &'a str,
            path: &'a str,
            date: &'a str,
        }
        let info = SessionCreated {
            title: note_title,
            project: &project_name,
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
        project: project_name.clone(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    // Link session to ticket if provided
    if let Some(ticket_slug) = ticket {
        link_session_to_ticket(config, &note_path, ticket_slug, state)?;
    }

    Ok(())
}

/// Link a session note to a ticket: update the ticket's sessions list and optionally its stage.
fn link_session_to_ticket(
    config: &Config,
    session_path: &std::path::Path,
    ticket_slug: &str,
    state: Option<&str>,
) -> Result<()> {
    // Find the ticket
    let ticket_info =
        crate::commands::ticket::find_ticket(config, ticket_slug, None)?.ok_or_else(|| {
            crate::error::TemperError::Vault(format!("ticket not found: {ticket_slug}"))
        })?;

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

    // Read the ticket file
    let ticket_path = config
        .tickets_dir
        .join(&ticket_info.project)
        .join(format!("{}.md", ticket_info.slug));
    let mut ticket_content = std::fs::read_to_string(&ticket_path)?;

    // Add/append to the sessions list in frontmatter
    if !session_id.is_empty() {
        if ticket_content.contains("\nsessions:") {
            // sessions key already exists — append to the list
            // Find the sessions: line and add a new entry after it (or after existing entries)
            let sessions_marker = "\nsessions:";
            if let Some(pos) = ticket_content.find(sessions_marker) {
                let after_marker = pos + sessions_marker.len();
                // Find the end of the sessions block: next non-indented line or closing ---
                // Find where to insert the new entry: right after "sessions:"
                let insert_pos = after_marker;
                let new_entry = format!("\n  - {session_id}");
                ticket_content.insert_str(insert_pos, &new_entry);
            }
        } else {
            // Insert sessions field before the closing --- of frontmatter
            // Find the second --- (closing frontmatter)
            let trimmed_start = if ticket_content.starts_with("---") {
                3
            } else {
                0
            };
            if let Some(close_pos) = ticket_content[trimmed_start..].find("\n---") {
                let insert_at = trimmed_start + close_pos;
                let new_field = format!("\nsessions:\n  - {session_id}");
                ticket_content.insert_str(insert_at, &new_field);
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
                ticket_content = vault::set_frontmatter_field(&ticket_content, "branch", &branch);
            }
        }
    }

    // Optionally update the ticket stage
    if let Some(s) = state {
        let valid_stages = ["backlog", "in-progress", "done", "cancelled"];
        if !valid_stages.contains(&s) {
            return Err(crate::error::TemperError::Vault(format!(
                "invalid stage: {s}. Must be one of: {}",
                valid_stages.join(", ")
            )));
        }
        ticket_content = vault::set_frontmatter_field(&ticket_content, "stage", s);
    }

    std::fs::write(&ticket_path, &ticket_content)?;
    Ok(())
}

/// List recent sessions, optionally filtered by project.
///
/// Scans `sessions_dir`, parses frontmatter for date, sorts by date descending,
/// displays up to 20 entries.
pub fn list(config: &Config, project: Option<&str>, format: &str) -> Result<()> {
    let sessions_root = &config.sessions_dir;

    if !sessions_root.exists() {
        output::warning("No sessions directory found.");
        return Ok(());
    }

    let mut entries: Vec<SessionEntry> = Vec::new();

    // Scan project subdirectories (or the root if flat)
    for proj_entry in std::fs::read_dir(sessions_root)? {
        let proj_entry = proj_entry?;
        let proj_path = proj_entry.path();

        if proj_path.is_dir() {
            let proj_name = proj_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Filter by project if specified
            if let Some(filter) = project {
                if proj_name != filter {
                    continue;
                }
            }

            collect_sessions(&proj_path, &proj_name, &mut entries)?;
        } else if proj_path.extension().is_some_and(|e| e == "md") {
            // Flat session files at root level
            if project.is_none() {
                let proj_name = "general".to_string();
                add_session_entry(&proj_path, &proj_name, &mut entries);
            }
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

    output::plain(format!("{:<12} {:<20} Title", "Date", "Project"));
    output::dim("-".repeat(60));
    for entry in &entries {
        output::plain(format!(
            "{:<12} {:<20} {}",
            entry.date, entry.project, entry.title
        ));
    }

    Ok(())
}

#[derive(Serialize)]
struct SessionEntry {
    date: String,
    project: String,
    title: String,
}

fn collect_sessions(
    dir: &std::path::Path,
    project: &str,
    entries: &mut Vec<SessionEntry>,
) -> Result<()> {
    for file_entry in std::fs::read_dir(dir)? {
        let file_entry = file_entry?;
        let path = file_entry.path();
        if path.extension().is_some_and(|e| e == "md") {
            add_session_entry(&path, project, entries);
        }
    }
    Ok(())
}

fn add_session_entry(path: &std::path::Path, project: &str, entries: &mut Vec<SessionEntry>) {
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
        project: project.to_string(),
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

/// Replace body content of a markdown note, preserving frontmatter.
fn replace_body(existing: &str, new_body: &str) -> String {
    let trimmed = existing.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---") {
        if let Some(end) = after_open.find("---") {
            let frontmatter_end = 3 + end + 3;
            let frontmatter = &trimmed[..frontmatter_end];
            return format!("{frontmatter}\n\n{new_body}");
        }
    }
    // No frontmatter: just replace entirely
    new_body.to_string()
}

/// Return path that would be used for a session note (for testing/preview).
#[allow(dead_code)]
pub fn session_path(config: &Config, project: &str, title: &str) -> PathBuf {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let filename = format!("{today} \u{2014} {title}.md");
    config.sessions_dir.join(project).join(filename)
}
