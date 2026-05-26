use crate::config::Config;
use crate::discovery::Event;
use crate::error::{Result, TemperError};
use crate::output;

/// Extract the context field from any Event variant.
pub fn event_context(event: &Event) -> &str {
    match event {
        Event::ResourceCreate { context, .. } | Event::ResourceUpdate { context, .. } => context,
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
        Event::ResourceCreate {
            ts,
            doc_type,
            title,
            context,
            ..
        } => format!("{ts}  {context:<12}  resource_create  {doc_type}: {title}"),
        Event::ResourceUpdate {
            ts,
            doc_type,
            slug,
            context,
        } => format!("{ts}  {context:<12}  resource_update  {doc_type}: {slug}"),
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
