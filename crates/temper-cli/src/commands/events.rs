use crate::config::Config;
use crate::discovery::Event;
use crate::error::{Result, TemperError};
use crate::output;

/// Extract the context field from any Event variant.
pub fn event_context(event: &Event) -> &str {
    match event {
        Event::NoteCreate { context, .. } => context,
        Event::TaskCreate { context, .. }
        | Event::TaskMove { context, .. }
        | Event::TaskDone { context, .. }
        | Event::GoalCreate { context, .. }
        | Event::GoalUpdate { context, .. } => context,
        Event::Normalize { project, .. } => project.as_deref().unwrap_or("general"),
        Event::ResourceCreate { context, .. } => context,
    }
}

/// Load events from events.jsonl, newest first, with optional project filter and limit.
pub fn load_events(config: &Config, project: Option<&str>, limit: usize) -> Result<Vec<Event>> {
    let log_path = config.state_dir.join("events.jsonl");
    if !log_path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&log_path)
        .map_err(|e| TemperError::Vault(format!("reading events.jsonl: {e}")))?;

    let mut events: Vec<Event> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    // Reverse for newest-first
    events.reverse();

    // Filter by project if specified
    if let Some(p) = project {
        events.retain(|e| event_context(e) == p);
    }

    events.truncate(limit);
    Ok(events)
}

/// Format a single event as a human-readable line.
fn format_event(event: &Event) -> String {
    match event {
        Event::NoteCreate {
            ts,
            note_type,
            title,
            context,
            ..
        } => format!("{ts}  {context:<12}  note_create     {note_type}: {title}"),
        Event::TaskCreate {
            ts,
            context,
            task,
            title,
            ..
        } => format!("{ts}  {context:<12}  task_create     {task}: {title}"),
        Event::TaskMove {
            ts,
            context,
            task,
            from_stage,
            to_stage,
            ..
        } => {
            format!("{ts}  {context:<12}  task_move       {task}: {from_stage} \u{2192} {to_stage}")
        }
        Event::TaskDone {
            ts, context, task, ..
        } => format!("{ts}  {context:<12}  task_done       {task}"),
        Event::GoalCreate {
            ts,
            context,
            goal,
            title,
        } => format!("{ts}  {context:<12}  goal_create     {goal}: {title}"),
        Event::GoalUpdate {
            ts,
            context,
            goal,
            status,
        } => format!("{ts}  {context:<12}  goal_update     {goal} \u{2192} {status}"),
        Event::Normalize {
            ts,
            project,
            ids_backfilled,
            files_moved,
            stages_migrated,
            slugs_fixed,
            frontmatter_fixed,
        } => {
            let proj = project.as_deref().unwrap_or("general");
            format!("{ts}  {proj:<12}  normalize       ids:{ids_backfilled} moved:{files_moved} stages:{stages_migrated} slugs:{slugs_fixed} fm:{frontmatter_fixed}")
        }
        Event::ResourceCreate {
            ts,
            doc_type,
            title,
            context,
            ..
        } => format!("{ts}  {context:<12}  resource_create  {doc_type}: {title}"),
    }
}

/// Run the events command — print events to stdout.
pub fn run(config: &Config, project: Option<&str>, limit: usize, format: &str) -> Result<()> {
    let events = load_events(config, project, limit)?;

    if events.is_empty() {
        output::hint("No events found.");
        return Ok(());
    }

    match format {
        "json" => {
            for event in &events {
                println!("{}", serde_json::to_string(event).unwrap_or_default());
            }
        }
        _ => {
            for event in &events {
                output::plain(format_event(event));
            }
        }
    }

    Ok(())
}
