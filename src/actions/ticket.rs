use std::fs;

use chrono::Local;

use crate::actions::types::TicketInfo;
use crate::commands::milestone;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;

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
pub fn find_ticket(
    config: &Config,
    slug_or_suffix: &str,
    project: Option<&str>,
) -> Result<Option<TicketInfo>> {
    let all = load_tickets(config, project, None)?;
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
    scope: Option<&str>,
) -> Result<String> {
    // Ensure maintenance milestone exists if needed
    let ms_slug = match milestone_slug {
        Some(ms) => ms.to_string(),
        None => milestone::ensure_maintenance(config, project)?,
    };
    // Verify milestone exists and project matches
    if let Some(ms) = milestone::find_milestone(config, &ms_slug, None)? {
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

    // Validate scope if provided
    let valid_scopes = ["patch", "feature", "epic"];
    if let Some(sc) = scope {
        if !valid_scopes.contains(&sc) {
            return Err(TemperError::Vault(format!(
                "invalid scope: {sc}. Must be one of: {}",
                valid_scopes.join(", ")
            )));
        }
    }

    let date = Local::now().format("%Y-%m-%d").to_string();
    let slug_title = vault::slugify(title);
    let slug = format!("{date}-{slug_title}");
    let datetime = Local::now().to_rfc3339();
    let seq = next_seq(config, project, &ms_slug)?;
    let seq_str = seq.to_string();
    let id = crate::ids::generate_id();

    let templates_dir = templates_dir_str(config);
    let scope_str = scope.unwrap_or("null");
    let vars = vec![
        ("slug", slug.as_str()),
        ("project", project),
        ("milestone", ms_slug.as_str()),
        ("seq", seq_str.as_str()),
        ("datetime", datetime.as_str()),
        ("id", id.as_str()),
        ("scope", scope_str),
    ];
    let mut content = vault::render_template_with_vars(
        &config.vault_root,
        &templates_dir,
        "ticket",
        title,
        &vars,
    )?;

    if let Some(stdin_content) = vault::read_stdin_if_piped() {
        content.push_str(&stdin_content);
        content.push('\n');
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
        scope: scope.map(String::from),
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
    project: Option<&str>,
    scope: Option<&str>,
) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix, project)?
        .ok_or_else(|| TemperError::Vault(format!("ticket not found: {slug_or_suffix}")))?;

    let valid_stages = ["backlog", "in-progress", "done", "cancelled"];
    if let Some(s) = stage {
        if !valid_stages.contains(&s) {
            return Err(TemperError::Vault(format!(
                "invalid stage: {s}. Must be one of: {}",
                valid_stages.join(", ")
            )));
        }
    }

    let valid_scopes = ["patch", "feature", "epic"];
    if let Some(sc) = scope {
        if !valid_scopes.contains(&sc) {
            return Err(TemperError::Vault(format!(
                "invalid scope: {sc}. Must be one of: {}",
                valid_scopes.join(", ")
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
    let from_scope = ticket.scope.clone();

    if let Some(s) = stage {
        content = vault::set_frontmatter_field(&content, "stage", s);
    }

    let mut from_ms: Option<String> = None;
    let mut to_ms: Option<String> = None;
    if let Some(ms) = new_milestone {
        // Validate milestone exists and project matches
        let ms_info = milestone::find_milestone(config, ms, None)?
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

    if let Some(sc) = scope {
        content = vault::set_frontmatter_field(&content, "scope", sc);
    }

    let datetime = Local::now().to_rfc3339();
    content = vault::set_frontmatter_field(&content, "updated", &datetime);
    fs::write(&path, &content).map_err(|e| TemperError::Vault(e.to_string()))?;

    let to_scope = scope.map(String::from);
    let from_scope_for_event = if scope.is_some() { from_scope } else { None };

    let event = discovery::Event::TicketMove {
        ts: datetime,
        project: ticket.project,
        ticket: ticket.slug.clone(),
        from_stage: from_stage.clone(),
        to_stage: to_stage.to_string(),
        from_milestone: from_ms,
        to_milestone: to_ms,
        from_scope: from_scope_for_event,
        to_scope,
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
    project: Option<&str>,
) -> Result<()> {
    let ticket = find_ticket(config, slug_or_suffix, project)?
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
