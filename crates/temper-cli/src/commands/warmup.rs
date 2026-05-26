use serde::Serialize;
use temper_core::vault::Vault;

use crate::config::Config;
use crate::error::Result;
use crate::format::{render, OutputFormat};

const MAX_SESSION_LINES: usize = 500;

/// Structured session entry for JSON/Toon rendering.
#[derive(Debug, Serialize)]
pub(crate) struct WarmupSession {
    pub date: String,
    pub title: String,
}

/// In-progress task entry for JSON/Toon rendering.
#[derive(Debug, Serialize)]
pub(crate) struct WarmupTask {
    pub title: String,
    pub slug: String,
    pub mode: Option<String>,
    pub effort: Option<String>,
}

/// Full warmup result — serialized by `render()` for JSON and Toon outputs.
#[derive(Debug, Serialize)]
pub(crate) struct WarmupResult {
    pub project: String,
    pub recent_sessions: Vec<WarmupSession>,
    pub last_session_content: Option<String>,
    pub in_progress_tasks: Vec<WarmupTask>,
}

/// Run the warmup command — output a context primer for a new session.
pub fn run(config: &Config, project: Option<&str>, format: OutputFormat) -> Result<()> {
    let project_name = project.unwrap_or("general");
    let sessions = collect_recent_sessions(config, project_name, 5);
    let in_progress = collect_in_progress_tasks(config, project_name);

    let last_session_content = sessions.first().and_then(|(_, _, path)| {
        std::fs::read_to_string(path).ok().map(|content| {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_SESSION_LINES {
                lines[..MAX_SESSION_LINES].join("\n")
            } else {
                content
            }
        })
    });

    let result = WarmupResult {
        project: project_name.to_string(),
        recent_sessions: sessions
            .iter()
            .map(|(date, title, _)| WarmupSession {
                date: date.clone(),
                title: title.clone(),
            })
            .collect(),
        last_session_content,
        in_progress_tasks: in_progress
            .into_iter()
            .map(|(title, slug, mode, effort)| WarmupTask {
                title,
                slug,
                mode,
                effort,
            })
            .collect(),
    };

    let rendered = render(&result, format)?;
    println!("{rendered}");
    Ok(())
}

/// Collect recent session files for a project, sorted by date descending.
/// Returns (date, title, path) tuples.
fn collect_recent_sessions(
    config: &Config,
    project: &str,
    limit: usize,
) -> Vec<(String, String, std::path::PathBuf)> {
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(project);
    let sessions_dir = vault_layout.doc_type_dir(&owner, project, "session");
    if !sessions_dir.exists() {
        return vec![];
    }

    let mut entries: Vec<(String, String, std::path::PathBuf)> = Vec::new();

    if let Ok(dir) = std::fs::read_dir(&sessions_dir) {
        for entry in dir.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Parse date from filename: "YYYY-MM-DD — Title"
            let date = if stem.len() >= 10 {
                stem[..10].to_string()
            } else {
                "unknown".to_string()
            };

            let title = if let Some(pos) = stem.find(" \u{2014} ") {
                stem[pos + " \u{2014} ".len()..].to_string()
            } else {
                stem.clone()
            };

            entries.push((date, title, path));
        }
    }

    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.truncate(limit);
    entries
}

/// Collect in-progress tasks for a project.
/// Returns (title, slug, mode, effort) tuples.
fn collect_in_progress_tasks(
    config: &Config,
    project: &str,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let tasks = match crate::commands::task::load_tasks(config, Some(project), None) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    tasks
        .into_iter()
        .filter(|t| t.stage == "in-progress")
        .map(|t| (t.title, t.slug, t.mode, t.effort))
        .collect()
}
