use crate::config::Config;
use crate::error::{Result, TemperError};
use crate::output;

// Re-export data types and functions from the actions layer
pub use crate::actions::ticket::{create, done, find_ticket, load_tickets, move_ticket, next_seq};
pub use crate::actions::types::TicketInfo;

/// Show a single ticket's raw markdown content.
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    project: Option<&str>,
    format: &str,
) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix, project)?
        .ok_or_else(|| TemperError::Vault(format!("ticket not found: {slug_or_suffix}")))?;
    if format == "json" {
        let json = serde_json::to_string_pretty(&ticket)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    let path = config
        .tickets_dir
        .join(&ticket.project)
        .join(format!("{}.md", ticket.slug));
    let content = std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    print!("{content}");
    Ok(())
}

/// List tickets grouped by milestone.
pub fn list(
    config: &Config,
    project: Option<&str>,
    milestone_slug: Option<&str>,
    format: &str,
) -> Result<()> {
    let tickets = load_tickets(config, project, milestone_slug)?;
    if format == "json" {
        let json = serde_json::to_string_pretty(&tickets)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    if tickets.is_empty() {
        output::hint("No tickets found.");
        return Ok(());
    }
    // Group by milestone
    let mut by_milestone: std::collections::BTreeMap<String, Vec<&TicketInfo>> =
        std::collections::BTreeMap::new();
    for ticket in &tickets {
        by_milestone
            .entry(ticket.milestone.clone())
            .or_default()
            .push(ticket);
    }
    for (ms, tix) in &by_milestone {
        output::blank();
        output::header(format!("## {ms}"));
        for t in tix {
            output::plain(format!(
                "  {:>3}  [{:<10}]  {} ({})",
                t.seq, t.stage, t.title, t.project
            ));
        }
    }
    Ok(())
}
