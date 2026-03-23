use crate::config::Config;
use crate::discovery::Event;
use crate::error::{Result, TemperError};

/// Extract the project field from any Event variant.
pub fn event_project(event: &Event) -> &str {
    match event {
        Event::NoteCreate { project, .. }
        | Event::TicketCreate { project, .. }
        | Event::TicketMove { project, .. }
        | Event::TicketDone { project, .. }
        | Event::MilestoneCreate { project, .. }
        | Event::MilestoneUpdate { project, .. } => project,
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
        events.retain(|e| event_project(e) == p);
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
            project,
            ..
        } => format!("{ts}  {project:<12}  note_create     {note_type}: {title}"),
        Event::TicketCreate {
            ts,
            project,
            ticket,
            title,
            ..
        } => format!("{ts}  {project:<12}  ticket_create   {ticket}: {title}"),
        Event::TicketMove {
            ts,
            project,
            ticket,
            from_stage,
            to_stage,
            ..
        } => {
            format!("{ts}  {project:<12}  ticket_move     {ticket}: {from_stage} → {to_stage}")
        }
        Event::TicketDone {
            ts,
            project,
            ticket,
            ..
        } => format!("{ts}  {project:<12}  ticket_done     {ticket}"),
        Event::MilestoneCreate {
            ts,
            project,
            milestone,
            title,
        } => format!("{ts}  {project:<12}  ms_create       {milestone}: {title}"),
        Event::MilestoneUpdate {
            ts,
            project,
            milestone,
            status,
        } => format!("{ts}  {project:<12}  ms_update       {milestone} → {status}"),
    }
}

/// Run the events command — print events to stdout.
pub fn run(config: &Config, project: Option<&str>, limit: usize, format: &str) -> Result<()> {
    let events = load_events(config, project, limit)?;

    if events.is_empty() {
        println!("No events found.");
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
                println!("{}", format_event(event));
            }
        }
    }

    Ok(())
}
