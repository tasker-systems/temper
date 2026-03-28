use crate::config::Config;
use crate::error::Result;
use crate::output;

// Re-export data types and pure functions from the actions layer
pub use crate::actions::milestone::{
    count_tickets_by_stage, ensure_maintenance, find_milestone, load_milestones, next_seq,
};
pub use crate::actions::types::MilestoneInfo;

/// Update a milestone's status with user-facing output.
pub fn update(config: &Config, slug: &str, status: &str, project: Option<&str>) -> Result<()> {
    crate::actions::milestone::update(config, slug, status, project)?;
    output::success(format!("Updated milestone: {slug} → {status}"));
    Ok(())
}

/// Create a new milestone, handling format/output.
pub fn create(
    config: &Config,
    project: &str,
    title: &str,
    slug: Option<&str>,
    format: &str,
) -> Result<String> {
    let slug = crate::actions::milestone::create(config, project, title, slug)?;
    if format == "json" {
        let seq = crate::actions::milestone::load_milestones(config, Some(project))?
            .into_iter()
            .find(|m| m.slug == slug)
            .map(|m| m.seq)
            .unwrap_or(0);
        let info = MilestoneInfo {
            title: title.to_string(),
            slug: slug.clone(),
            project: project.to_string(),
            seq,
            status: "active".to_string(),
        };
        let json = serde_json::to_string_pretty(&info).map_err(|e| {
            crate::error::TemperError::Vault(format!("json serialization failed: {e}"))
        })?;
        println!("{json}");
    } else {
        output::success(format!("Created milestone: {slug}"));
    }
    Ok(slug)
}

/// List milestones for a project with ticket counts (roadmap view).
pub fn list(config: &Config, project: &str, format: &str) -> Result<()> {
    let milestones = load_milestones(config, Some(project))?;
    if format == "json" {
        let json = serde_json::to_string_pretty(&milestones).map_err(|e| {
            crate::error::TemperError::Vault(format!("json serialization failed: {e}"))
        })?;
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
