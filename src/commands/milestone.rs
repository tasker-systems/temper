use std::fs;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MilestoneInfo {
    pub title: String,
    pub slug: String,
    pub project: String,
    pub seq: u32,
    pub status: String,
}

/// Load all milestones, optionally filtered by project, sorted by seq.
pub fn load_milestones(config: &Config, project: Option<&str>) -> Result<Vec<MilestoneInfo>> {
    let base = &config.milestones_dir;
    if !base.is_dir() {
        return Ok(vec![]);
    }
    let mut milestones = Vec::new();
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
            let info: MilestoneInfo = match serde_yaml::from_value(fm) {
                Ok(i) => i,
                Err(_) => continue,
            };
            milestones.push(info);
        }
    }
    milestones.sort_by_key(|m| m.seq);
    Ok(milestones)
}

/// Get the next seq value for a new milestone in a project (max seq + 10, minimum 10).
pub fn next_seq(config: &Config, project: &str) -> Result<u32> {
    let milestones = load_milestones(config, Some(project))?;
    let max_seq = milestones.iter().map(|m| m.seq).max().unwrap_or(0);
    Ok(max_seq + 10)
}

/// Find a milestone by slug, optionally scoped to a project.
pub fn find_milestone(
    config: &Config,
    slug: &str,
    project: Option<&str>,
) -> Result<Option<MilestoneInfo>> {
    let milestones = load_milestones(config, project)?;
    Ok(milestones.into_iter().find(|m| m.slug == slug))
}

/// Ensure the maintenance milestone exists for a project, creating it if missing.
pub fn ensure_maintenance(config: &Config, project: &str) -> Result<String> {
    let slug = format!("{project}-maintenance");
    let dir = config.milestones_dir.join(project);
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Ok(slug);
    }
    let templates_dir = config
        .templates_dir
        .strip_prefix(&config.vault_root)
        .unwrap_or(&config.templates_dir)
        .to_str()
        .unwrap_or("templates");
    let id = crate::ids::generate_id();
    let vars = vec![
        ("slug", slug.as_str()),
        ("project", project),
        ("seq", "0"),
        ("id", id.as_str()),
    ];
    let content = vault::render_template_with_vars(
        &config.vault_root,
        templates_dir,
        "milestone",
        "Maintenance",
        &vars,
    )?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    let event = discovery::Event::MilestoneCreate {
        ts: Local::now().to_rfc3339(),
        project: project.to_string(),
        milestone: slug.clone(),
        title: "Maintenance".to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    Ok(slug)
}

/// Create a new milestone.
pub fn create(
    config: &Config,
    project: &str,
    title: &str,
    slug: Option<&str>,
    format: &str,
) -> Result<String> {
    let slug = match slug {
        Some(s) => s.to_string(),
        None => vault::slugify(title),
    };
    let dir = config.milestones_dir.join(project);
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Err(TemperError::Vault(format!(
            "milestone already exists: {slug}"
        )));
    }
    let seq = next_seq(config, project)?;
    let seq_str = seq.to_string();
    let id = crate::ids::generate_id();
    let templates_dir = config
        .templates_dir
        .strip_prefix(&config.vault_root)
        .unwrap_or(&config.templates_dir)
        .to_str()
        .unwrap_or("templates");
    let vars = vec![
        ("slug", slug.as_str()),
        ("project", project),
        ("seq", seq_str.as_str()),
        ("id", id.as_str()),
    ];
    let content = vault::render_template_with_vars(
        &config.vault_root,
        templates_dir,
        "milestone",
        title,
        &vars,
    )?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    let event = discovery::Event::MilestoneCreate {
        ts: Local::now().to_rfc3339(),
        project: project.to_string(),
        milestone: slug.clone(),
        title: title.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    if format == "json" {
        let info = MilestoneInfo {
            title: title.to_string(),
            slug: slug.clone(),
            project: project.to_string(),
            seq,
            status: "active".to_string(),
        };
        let json = serde_json::to_string_pretty(&info)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
    } else {
        output::success(format!("Created milestone: {slug}"));
    }
    Ok(slug)
}

/// Count tickets per milestone and stage by scanning ticket files directly.
fn count_tickets_by_stage(
    config: &Config,
    project: &str,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, usize>>> {
    let dir = config.tickets_dir.join(project);
    let mut counts: std::collections::HashMap<String, std::collections::HashMap<String, usize>> =
        std::collections::HashMap::new();
    if !dir.is_dir() {
        return Ok(counts);
    }
    for entry in fs::read_dir(&dir).map_err(|e| TemperError::Vault(e.to_string()))? {
        let entry = entry.map_err(|e| TemperError::Vault(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap_or_default();
        let fm = match vault::parse_frontmatter(&content) {
            Some(fm) => fm,
            None => continue,
        };
        let ms = fm
            .get("milestone")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let stage = fm
            .get("stage")
            .and_then(|v| v.as_str())
            .unwrap_or("backlog");
        *counts
            .entry(ms.to_string())
            .or_default()
            .entry(stage.to_string())
            .or_default() += 1;
    }
    Ok(counts)
}

/// List milestones for a project with ticket counts (roadmap view).
pub fn list(config: &Config, project: &str, format: &str) -> Result<()> {
    let milestones = load_milestones(config, Some(project))?;
    if format == "json" {
        let json = serde_json::to_string_pretty(&milestones)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    if milestones.is_empty() {
        output::hint(format!("No milestones for project: {project}"));
        return Ok(());
    }
    let ticket_counts = count_tickets_by_stage(config, project)?;
    let project_title = project.chars().next().unwrap().to_uppercase().to_string() + &project[1..];
    output::header(format!("{project_title} Roadmap"));
    output::plain("─".repeat(14));
    // Partition: non-zero seq first (sorted by seq), then zero-seq (maintenance) pinned to bottom
    let (maintenance, regular): (Vec<_>, Vec<_>) = milestones.iter().partition(|m| m.seq == 0);
    let ordered: Vec<_> = regular.into_iter().chain(maintenance).collect();
    for ms in &ordered {
        let ms_counts = ticket_counts.get(&ms.slug);
        let mut stage_parts: Vec<String> = Vec::new();
        for stage in &[
            "backlog",
            "brainstorm",
            "design",
            "plan",
            "implement",
            "done",
        ] {
            let count = ms_counts.and_then(|c| c.get(*stage)).copied().unwrap_or(0);
            if count > 0 {
                stage_parts.push(format!("{count} {stage}"));
            }
        }
        let counts_str = if stage_parts.is_empty() {
            "empty".to_string()
        } else {
            stage_parts.join(" · ")
        };
        let seq_display = if ms.seq == 0 {
            "  ·".to_string()
        } else {
            format!("{:>3}", ms.seq)
        };
        output::plain(format!(
            " {seq_display}  {:<24} [{:<9}]   {counts_str}",
            ms.title, ms.status
        ));
    }
    Ok(())
}

/// Update a milestone's status.
pub fn update(config: &Config, slug: &str, status: &str, project: Option<&str>) -> Result<()> {
    let valid_statuses = ["active", "completed", "paused", "cancelled"];
    if !valid_statuses.contains(&status) {
        return Err(TemperError::Vault(format!(
            "invalid status: {status}. Must be one of: {}",
            valid_statuses.join(", ")
        )));
    }
    let info = find_milestone(config, slug, project)?
        .ok_or_else(|| TemperError::Vault(format!("milestone not found: {slug}")))?;
    let path = config
        .milestones_dir
        .join(&info.project)
        .join(format!("{slug}.md"));
    if !path.exists() {
        return Err(TemperError::Vault(format!("milestone not found: {slug}")));
    }
    let content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let updated = vault::set_frontmatter_field(&content, "status", status);
    fs::write(&path, updated).map_err(|e| TemperError::Vault(e.to_string()))?;
    let event = discovery::Event::MilestoneUpdate {
        ts: Local::now().to_rfc3339(),
        project: info.project,
        milestone: slug.to_string(),
        status: status.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!("Updated milestone: {slug} → {status}"));
    Ok(())
}
