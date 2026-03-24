use std::fs;
use std::io::Read as IoRead;

use chrono::Local;
use serde::Deserialize;

use crate::commands::milestone;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;

#[derive(Debug, Clone, Deserialize)]
pub struct TicketInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub milestone: String,
    pub stage: String,
    pub seq: u32,
    #[allow(dead_code)]
    pub branch: Option<String>,
    #[allow(dead_code)]
    pub pr: Option<String>,
}

/// Load all tickets, optionally filtered by project and/or milestone.
pub fn load_tickets(
    config: &Config,
    project: Option<&str>,
    milestone_slug: Option<&str>,
) -> Result<Vec<TicketInfo>> {
    let base = &config.tickets_dir;
    if !base.is_dir() {
        return Ok(vec![]);
    }
    let mut tickets = Vec::new();
    let dirs: Vec<_> = if let Some(p) = project {
        let d = base.join(p);
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        fs::read_dir(base)
            .map_err(|e| TemperError::Vault(e.to_string()))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    };
    for dir in dirs {
        for entry in fs::read_dir(&dir).map_err(|e| TemperError::Vault(e.to_string()))? {
            let entry = entry.map_err(|e| TemperError::Vault(e.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let content = fs::read_to_string(&path)
                .map_err(|e| TemperError::Vault(format!("reading {}: {e}", path.display())))?;
            let fm = match vault::parse_frontmatter(&content) {
                Some(fm) => fm,
                None => continue,
            };
            let info: TicketInfo = match serde_yaml::from_value(fm) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if let Some(ms) = milestone_slug {
                if info.milestone != ms {
                    continue;
                }
            }
            tickets.push(info);
        }
    }
    tickets.sort_by_key(|t| t.seq);
    Ok(tickets)
}

/// Get the next seq value for a new ticket in a milestone.
pub fn next_seq(config: &Config, project: &str, milestone_slug: &str) -> Result<u32> {
    let tickets = load_tickets(config, Some(project), Some(milestone_slug))?;
    let max_seq = tickets.iter().map(|t| t.seq).max().unwrap_or(0);
    Ok(max_seq + 10)
}

/// Find a ticket by exact slug or unambiguous suffix match.
pub fn find_ticket(config: &Config, slug_or_suffix: &str) -> Result<Option<TicketInfo>> {
    let all = load_tickets(config, None, None)?;
    // Exact match first
    if let Some(t) = all.iter().find(|t| t.slug == slug_or_suffix) {
        return Ok(Some(t.clone()));
    }
    // Suffix match
    let matches: Vec<_> = all
        .iter()
        .filter(|t| t.slug.ends_with(slug_or_suffix))
        .collect();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches[0].clone())),
        _ => Err(TemperError::Vault(format!(
            "ambiguous slug suffix '{slug_or_suffix}', matches: {}",
            matches
                .iter()
                .map(|t| t.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn templates_dir_str(config: &Config) -> String {
    config
        .templates_dir
        .strip_prefix(&config.vault_root)
        .unwrap_or(&config.templates_dir)
        .to_str()
        .unwrap_or("templates")
        .to_string()
}

/// Create a new ticket.
pub fn create(
    config: &Config,
    project: &str,
    title: &str,
    milestone_slug: Option<&str>,
    stdin: bool,
) -> Result<String> {
    // Ensure maintenance milestone exists if needed
    let ms_slug = match milestone_slug {
        Some(ms) => ms.to_string(),
        None => milestone::ensure_maintenance(config, project)?,
    };
    // Verify milestone exists and project matches
    if let Some(ms) = milestone::find_milestone(config, &ms_slug)? {
        if ms.project != project {
            return Err(TemperError::Vault(format!(
                "milestone '{}' belongs to project '{}', not '{project}'",
                ms_slug, ms.project
            )));
        }
    } else if milestone_slug.is_some() {
        return Err(TemperError::Vault(format!(
            "milestone not found: {ms_slug}"
        )));
    }

    let date = Local::now().format("%Y-%m-%d").to_string();
    let slug_title = vault::slugify(title);
    let slug = format!("{date}-{slug_title}");
    let datetime = Local::now().to_rfc3339();
    let seq = next_seq(config, project, &ms_slug)?;
    let seq_str = seq.to_string();

    let templates_dir = templates_dir_str(config);
    let vars = vec![
        ("slug", slug.as_str()),
        ("project", project),
        ("milestone", ms_slug.as_str()),
        ("seq", seq_str.as_str()),
        ("datetime", datetime.as_str()),
    ];
    let mut content = vault::render_template_with_vars(
        &config.vault_root,
        &templates_dir,
        "ticket",
        title,
        &vars,
    )?;

    if stdin {
        let mut stdin_content = String::new();
        std::io::stdin()
            .read_to_string(&mut stdin_content)
            .map_err(|e| TemperError::Vault(format!("reading stdin: {e}")))?;
        if !stdin_content.is_empty() {
            content.push_str(&stdin_content);
            content.push('\n');
        }
    }

    let dir = config.tickets_dir.join(project);
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let path = dir.join(format!("{slug}.md"));
    vault::write_note(&path, &content)?;

    let event = discovery::Event::TicketCreate {
        ts: datetime,
        project: project.to_string(),
        ticket: slug.clone(),
        milestone: ms_slug,
        title: title.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!("Created ticket: {slug}"));
    Ok(slug)
}

/// Move a ticket to a new stage and/or milestone.
pub fn move_ticket(
    config: &Config,
    slug_or_suffix: &str,
    stage: Option<&str>,
    new_milestone: Option<&str>,
) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix)?
        .ok_or_else(|| TemperError::Vault(format!("ticket not found: {slug_or_suffix}")))?;

    let valid_stages = [
        "backlog",
        "brainstorm",
        "design",
        "plan",
        "implement",
        "done",
    ];
    if let Some(s) = stage {
        if !valid_stages.contains(&s) {
            return Err(TemperError::Vault(format!(
                "invalid stage: {s}. Must be one of: {}",
                valid_stages.join(", ")
            )));
        }
    }

    let path = config
        .tickets_dir
        .join(&ticket.project)
        .join(format!("{}.md", ticket.slug));
    let mut content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;

    let from_stage = ticket.stage.clone();
    let to_stage = stage.unwrap_or(&from_stage);

    if let Some(s) = stage {
        content = vault::set_frontmatter_field(&content, "stage", s);
    }

    let mut from_ms: Option<String> = None;
    let mut to_ms: Option<String> = None;
    if let Some(ms) = new_milestone {
        // Validate milestone exists and project matches
        let ms_info = milestone::find_milestone(config, ms)?
            .ok_or_else(|| TemperError::Vault(format!("milestone not found: {ms}")))?;
        if ms_info.project != ticket.project {
            return Err(TemperError::Vault(format!(
                "milestone '{}' belongs to project '{}', not '{}'",
                ms, ms_info.project, ticket.project
            )));
        }
        from_ms = Some(ticket.milestone.clone());
        to_ms = Some(ms.to_string());
        content = vault::set_frontmatter_field(&content, "milestone", ms);
        // Assign new seq at end of target milestone
        let new_seq = next_seq(config, &ticket.project, ms)?;
        content = vault::set_frontmatter_field(&content, "seq", &new_seq.to_string());
    }

    let datetime = Local::now().to_rfc3339();
    content = vault::set_frontmatter_field(&content, "updated", &datetime);
    fs::write(&path, &content).map_err(|e| TemperError::Vault(e.to_string()))?;

    let event = discovery::Event::TicketMove {
        ts: datetime,
        project: ticket.project,
        ticket: ticket.slug.clone(),
        from_stage: from_stage.clone(),
        to_stage: to_stage.to_string(),
        from_milestone: from_ms,
        to_milestone: to_ms,
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!(
        "Moved ticket {}: {from_stage} → {to_stage}",
        ticket.slug
    ));
    Ok(())
}

/// Mark a ticket as done with branch and PR info.
pub fn done(
    config: &Config,
    slug_or_suffix: &str,
    branch: Option<&str>,
    pr: Option<&str>,
) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix)?
        .ok_or_else(|| TemperError::Vault(format!("ticket not found: {slug_or_suffix}")))?;

    let path = config
        .tickets_dir
        .join(&ticket.project)
        .join(format!("{}.md", ticket.slug));
    let mut content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;

    let datetime = Local::now().to_rfc3339();
    content = vault::set_frontmatter_field(&content, "stage", "done");
    content = vault::set_frontmatter_field(&content, "updated", &datetime);
    if let Some(b) = branch {
        content = vault::set_frontmatter_field(&content, "branch", b);
    }
    if let Some(p) = pr {
        content = vault::set_frontmatter_field(&content, "pr", p);
    }
    fs::write(&path, &content).map_err(|e| TemperError::Vault(e.to_string()))?;

    let event = discovery::Event::TicketDone {
        ts: datetime,
        project: ticket.project,
        ticket: ticket.slug.clone(),
        branch: branch.map(String::from),
        pr: pr.map(String::from),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!("Completed ticket: {}", ticket.slug));
    Ok(())
}

/// Show a single ticket's raw markdown content.
pub fn show(config: &Config, slug_or_suffix: &str) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix)?
        .ok_or_else(|| TemperError::Vault(format!("ticket not found: {slug_or_suffix}")))?;
    let path = config
        .tickets_dir
        .join(&ticket.project)
        .join(format!("{}.md", ticket.slug));
    let content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    print!("{content}");
    Ok(())
}

/// List tickets grouped by milestone.
pub fn list(config: &Config, project: Option<&str>, milestone_slug: Option<&str>) -> Result<()> {
    let tickets = load_tickets(config, project, milestone_slug)?;
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
pub fn board(config: &Config, project: &str, milestone_filter: Option<&str>) -> Result<()> {
    let milestones = milestone::load_milestones(config, Some(project))?;
    let tickets = load_tickets(config, Some(project), None)?;
    let stages = [
        "backlog",
        "brainstorm",
        "design",
        "plan",
        "implement",
        "done",
    ];

    let project_title = project.chars().next().unwrap().to_uppercase().to_string() + &project[1..];

    // Terminal output
    output::header(format!("{project_title} Board"));
    output::plain("═".repeat(68));

    let filtered_milestones: Vec<_> = if let Some(ms) = milestone_filter {
        milestones.iter().filter(|m| m.slug == ms).collect()
    } else {
        milestones.iter().filter(|m| m.status == "active").collect()
    };

    for ms in &filtered_milestones {
        let ms_tickets: Vec<_> = tickets.iter().filter(|t| t.milestone == ms.slug).collect();
        if ms_tickets.is_empty() && milestone_filter.is_none() {
            continue;
        }
        output::plain(format!(
            " {:<16}│ {:<16}│ {:<16}│ {:<8}│ {:<16}│ Done",
            "Backlog", "Brainstorm", "Design", "Plan", "Implement"
        ));
        output::plain(format!(
            "{}┼{}┼{}┼{}┼{}┼{}",
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(9),
            "─".repeat(17),
            "─".repeat(9)
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
                    let width = match i {
                        3 => 8,  // Plan (was index 2)
                        5 => 9,  // Done (was index 4)
                        _ => 16, // Backlog, Brainstorm, Design, Implement
                    };
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
                "{}│{}│{}│{}│{}│{}",
                cells[0], cells[1], cells[2], cells[3], cells[4], cells[5]
            ));
        }

        output::plain(format!(
            "{}┴{}┴{}┴{}┴{}┴{}",
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(17),
            "─".repeat(9),
            "─".repeat(17),
            "─".repeat(9)
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
        Local::now().to_rfc3339()
    );

    for ms in &filtered_milestones {
        let ms_tickets: Vec<_> = tickets.iter().filter(|t| t.milestone == ms.slug).collect();
        md.push_str(&format!("\n## {}\n\n", ms.title));
        md.push_str("| Backlog | Brainstorm | Design | Plan | Implement | Done |\n");
        md.push_str("|---------|------------|--------|------|-----------|------|\n");

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
    fs::create_dir_all(&board_dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let board_path = board_dir.join(format!("{project}.md"));
    fs::write(&board_path, &md).map_err(|e| TemperError::Vault(e.to_string()))?;
    output::dim(format!("Board written to boards/{project}.md"));
    Ok(())
}
