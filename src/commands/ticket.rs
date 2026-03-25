use serde::Serialize;

use crate::commands::milestone;
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

/// Generate board view: terminal output + markdown file.
pub fn board(
    config: &Config,
    project: &str,
    milestone_filter: Option<&str>,
    format: &str,
) -> Result<()> {
    let milestones = milestone::load_milestones(config, Some(project))?;
    let tickets = load_tickets(config, Some(project), None)?;
    let stages = ["backlog", "in-progress", "done", "cancelled"];

    let filtered_milestones_for_json: Vec<_> = if let Some(ms) = milestone_filter {
        milestones.iter().filter(|m| m.slug == ms).collect()
    } else {
        milestones.iter().filter(|m| m.status == "active").collect()
    };

    if format == "json" {
        #[derive(Serialize)]
        struct BoardMilestone<'a> {
            milestone: &'a str,
            title: &'a str,
            tickets: Vec<&'a TicketInfo>,
        }
        let board_data: Vec<BoardMilestone> = filtered_milestones_for_json
            .iter()
            .map(|ms| {
                let ms_tickets: Vec<&TicketInfo> =
                    tickets.iter().filter(|t| t.milestone == ms.slug).collect();
                BoardMilestone {
                    milestone: &ms.slug,
                    title: &ms.title,
                    tickets: ms_tickets,
                }
            })
            .collect();
        let json = serde_json::to_string_pretty(&board_data)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }

    let project_title = project.chars().next().unwrap().to_uppercase().to_string() + &project[1..];

    // Terminal output
    output::header(format!("{project_title} Board"));
    output::plain("═".repeat(68));

    for ms in &filtered_milestones_for_json {
        let ms_tickets: Vec<_> = tickets.iter().filter(|t| t.milestone == ms.slug).collect();
        if ms_tickets.is_empty() && milestone_filter.is_none() {
            continue;
        }
        output::plain(format!(
            " {:<20}│ {:<20}│ {:<20}│ Cancelled",
            "Backlog", "In Progress", "Done"
        ));
        output::plain(format!(
            "{}┼{}┼{}┼{}",
            "─".repeat(21),
            "─".repeat(21),
            "─".repeat(21),
            "─".repeat(17)
        ));

        let max_rows = stages
            .iter()
            .map(|s| ms_tickets.iter().filter(|t| t.stage == *s).count())
            .max()
            .unwrap_or(0);

        let by_stage: Vec<Vec<&TicketInfo>> = stages
            .iter()
            .map(|s| {
                ms_tickets
                    .iter()
                    .filter(|t| t.stage == *s)
                    .copied()
                    .collect()
            })
            .collect();

        for row in 0..max_rows.max(1) {
            let cells: Vec<String> = by_stage
                .iter()
                .enumerate()
                .map(|(i, stage_tickets)| {
                    let width = if i == 3 { 16 } else { 20 };
                    if let Some(t) = stage_tickets.get(row) {
                        let name = if t.title.len() > width {
                            format!("{}…", &t.title[..width - 1])
                        } else {
                            t.title.clone()
                        };
                        format!(" {:<width$}", name)
                    } else {
                        format!(" {:<width$}", "")
                    }
                })
                .collect();
            output::plain(format!(
                "{}│{}│{}│{}",
                cells[0], cells[1], cells[2], cells[3]
            ));
        }

        output::plain(format!(
            "{}┴{}┴{}┴{}",
            "─".repeat(21),
            "─".repeat(21),
            "─".repeat(21),
            "─".repeat(17)
        ));
        let mut stage_counts: Vec<String> = Vec::new();
        for stage in &stages {
            let count = ms_tickets.iter().filter(|t| t.stage == *stage).count();
            if count > 0 {
                stage_counts.push(format!("{count} {stage}"));
            }
        }
        let counts_str = stage_counts.join(" · ");
        output::plain(format!(" Milestone: {} ({counts_str})", ms.title));
        output::blank();
    }

    // Generate markdown board
    let mut md = format!(
        "---\ntype: board\nproject: {project}\ngenerated: {}\n---\n\n# {project_title} Board\n",
        chrono::Local::now().to_rfc3339()
    );

    for ms in &filtered_milestones_for_json {
        let ms_tickets: Vec<_> = tickets.iter().filter(|t| t.milestone == ms.slug).collect();
        md.push_str(&format!("\n## {}\n\n", ms.title));
        md.push_str("| Backlog | In Progress | Done | Cancelled |\n");
        md.push_str("|---------|-------------|------|----------|\n");

        let by_stage: Vec<Vec<&TicketInfo>> = stages
            .iter()
            .map(|s| {
                ms_tickets
                    .iter()
                    .filter(|t| t.stage == *s)
                    .copied()
                    .collect()
            })
            .collect();
        let max_rows = by_stage.iter().map(|s| s.len()).max().unwrap_or(0);

        for row in 0..max_rows.max(1) {
            let cells: Vec<String> = by_stage
                .iter()
                .map(|stage_tickets| {
                    if let Some(t) = stage_tickets.get(row) {
                        format!("[[{}|{}]]", t.slug, t.title)
                    } else {
                        String::new()
                    }
                })
                .collect();
            md.push_str(&format!("| {} |\n", cells.join(" | ")));
        }
    }

    let board_dir = config.vault_root.join("boards");
    std::fs::create_dir_all(&board_dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let board_path = board_dir.join(format!("{project}.md"));
    std::fs::write(&board_path, &md).map_err(|e| TemperError::Vault(e.to_string()))?;
    output::dim(format!("Board written to boards/{project}.md"));
    Ok(())
}
