use crate::commands::events;
use crate::config::Config;
use crate::error::Result;

const MAX_SESSION_LINES: usize = 500;

/// Run the warmup command — output a context primer for a new session.
pub fn run(config: &Config, project: Option<&str>, format: &str) -> Result<()> {
    let project_name = project.unwrap_or("general");

    match format {
        "json" => run_json(config, project_name),
        _ => run_text(config, project_name),
    }
}

fn run_text(config: &Config, project: &str) -> Result<()> {
    println!("# Session Context: {project}");
    println!();

    // Section 1: Recent sessions
    println!("## Recent Sessions");
    println!();
    let sessions = collect_recent_sessions(config, project, 3);
    if sessions.is_empty() {
        println!("No recent sessions.");
    } else {
        for (date, title, _path) in &sessions {
            println!("- {date}: {title}");
        }
    }
    println!();

    // Section: In-progress tasks
    let in_progress = collect_in_progress_tasks(config, project);
    if !in_progress.is_empty() {
        println!("## In-Progress Tasks");
        println!();
        for (title, slug, mode, effort) in &in_progress {
            let mode_label = mode.as_deref().unwrap_or("no-mode");
            let effort_label = effort.as_deref().unwrap_or("no-effort");
            println!("- [{mode_label}/{effort_label}] {title} ({slug})");
        }
        println!();
    }

    // Section 2: Last session content
    if let Some((_date, _title, path)) = sessions.first() {
        println!("## Last Session");
        println!();
        if let Ok(content) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_SESSION_LINES {
                for line in &lines[..MAX_SESSION_LINES] {
                    println!("{line}");
                }
                println!();
                println!(
                    "... (truncated at {MAX_SESSION_LINES} lines, see full note at {})",
                    path.display()
                );
            } else {
                print!("{content}");
            }
        }
        println!();
    }

    // Section 3: Recent events
    println!("## Recent Events");
    println!();
    let recent_events = events::load_events(config, Some(project), 15)?;
    if recent_events.is_empty() {
        println!("No recent events.");
    } else {
        for event in &recent_events {
            println!("{}", format_event_brief(event));
        }
    }

    Ok(())
}

fn run_json(config: &Config, project: &str) -> Result<()> {
    let sessions = collect_recent_sessions(config, project, 3);
    let recent_events = events::load_events(config, Some(project), 15)?;
    let in_progress = collect_in_progress_tasks(config, project);
    let in_progress_json: Vec<_> = in_progress
        .iter()
        .map(|(title, slug, mode, effort)| {
            serde_json::json!({
                "title": title,
                "slug": slug,
                "mode": mode,
                "effort": effort,
            })
        })
        .collect();

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

    let output = serde_json::json!({
        "project": project,
        "recent_sessions": sessions.iter().map(|(date, title, _)| {
            serde_json::json!({"date": date, "title": title})
        }).collect::<Vec<_>>(),
        "last_session_content": last_session_content,
        "recent_events": recent_events.iter().map(|e| {
            serde_json::to_value(e).unwrap_or_default()
        }).collect::<Vec<_>>(),
        "in_progress_tasks": in_progress_json,
    });

    println!(
        "{}",
        serde_json::to_string_pretty(&output).unwrap_or_default()
    );
    Ok(())
}

/// Collect recent session files for a project, sorted by date descending.
/// Returns (date, title, path) tuples.
fn collect_recent_sessions(
    config: &Config,
    project: &str,
    limit: usize,
) -> Vec<(String, String, std::path::PathBuf)> {
    let sessions_dir = config.sessions_dir.join(project);
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

/// Brief event formatting for warmup output.
fn format_event_brief(event: &crate::discovery::Event) -> String {
    use crate::discovery::Event;
    match event {
        Event::NoteCreate {
            ts,
            note_type,
            title,
            ..
        } => {
            let date = &ts[..10];
            format!("  {date}  created {note_type}: {title}")
        }
        Event::TaskCreate {
            ts, task, title, ..
        } => {
            let date = &ts[..10];
            format!("  {date}  created task: {title} ({task})")
        }
        Event::TaskMove {
            ts,
            task,
            from_stage,
            to_stage,
            ..
        } => {
            let date = &ts[..10];
            format!("  {date}  moved {task}: {from_stage} \u{2192} {to_stage}")
        }
        Event::TaskDone { ts, task, .. } => {
            let date = &ts[..10];
            format!("  {date}  completed {task}")
        }
        Event::GoalCreate {
            ts, goal, title, ..
        } => {
            let date = &ts[..10];
            format!("  {date}  created goal: {title} ({goal})")
        }
        Event::GoalUpdate {
            ts, goal, status, ..
        } => {
            let date = &ts[..10];
            format!("  {date}  goal {goal} \u{2192} {status}")
        }
        Event::Normalize { ts, .. } => {
            let date = &ts[..10];
            format!("  {date}  normalize")
        }
    }
}
