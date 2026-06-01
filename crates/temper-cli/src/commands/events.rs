use crate::config::Config;
use crate::discovery::Event;
use crate::error::{Result, TemperError};

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

/// Run the events command — print events to stdout.
pub fn run(
    config: &Config,
    project: Option<&str>,
    limit: usize,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let events = load_events(config, project, limit)?;
    let rendered = crate::format::render(&events, fmt)?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_events() -> Vec<Event> {
        vec![
            Event::ResourceCreate {
                ts: "2026-05-26T12:00:00Z".to_string(),
                doc_type: "task".to_string(),
                title: "Sample".to_string(),
                path: "task/sample.md".to_string(),
                context: "temper".to_string(),
            },
            Event::ResourceUpdate {
                ts: "2026-05-26T12:01:00Z".to_string(),
                doc_type: "task".to_string(),
                slug: "sample".to_string(),
                context: "temper".to_string(),
            },
        ]
    }

    #[test]
    fn render_events_json_is_array() {
        let events = fixture_events();
        let out =
            crate::format::render(&events, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("resource_create"), "json: {out}");
        assert!(out.contains("Sample"), "json: {out}");
    }

    #[test]
    fn render_events_toon_includes_event_marker() {
        let events = fixture_events();
        let out =
            crate::format::render(&events, crate::format::OutputFormat::Toon).expect("toon render");
        // Contains-check on stable field names.
        assert!(
            out.contains("ts") || out.contains("doc_type"),
            "toon: {out}"
        );
        assert!(out.contains("temper"), "toon: {out}");
    }

    #[test]
    fn render_empty_events_json_is_empty_array() {
        let events: Vec<Event> = vec![];
        let out =
            crate::format::render(&events, crate::format::OutputFormat::Json).expect("json render");
        assert_eq!(out.trim(), "[]");
    }
}
