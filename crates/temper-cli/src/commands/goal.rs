use crate::config::Config;
use crate::error::Result;
use crate::output;

// Re-export data types and pure functions from the actions layer
pub use crate::actions::goal::{
    count_tasks_by_stage, ensure_maintenance, find_goal, load_goals, next_seq,
};
pub use crate::actions::types::GoalInfo;

/// Update a goal's status with user-facing output.
pub fn update(config: &Config, slug: &str, status: &str, context: Option<&str>) -> Result<()> {
    crate::actions::goal::update(config, slug, status, context)?;
    output::success(format!("Updated goal: {slug} → {status}"));
    Ok(())
}

/// Create a new goal, handling format/output.
pub fn create(
    config: &Config,
    context: &str,
    title: &str,
    slug: Option<&str>,
    format: &str,
) -> Result<String> {
    let slug = crate::actions::goal::create(config, context, title, slug)?;
    if format == "json" {
        let seq = crate::actions::goal::load_goals(config, Some(context))?
            .into_iter()
            .find(|m| m.slug == slug)
            .map(|m| m.seq)
            .unwrap_or(0);
        let info = GoalInfo {
            title: title.to_string(),
            slug: slug.clone(),
            context: context.to_string(),
            seq,
            status: "active".to_string(),
        };
        let json = serde_json::to_string_pretty(&info).map_err(|e| {
            crate::error::TemperError::Vault(format!("json serialization failed: {e}"))
        })?;
        println!("{json}");
    } else {
        output::success(format!("Created goal: {slug}"));
    }
    Ok(slug)
}

/// List goals for a context with task counts (roadmap view).
pub fn list(config: &Config, context: &str, format: &str) -> Result<()> {
    let goals = load_goals(config, Some(context))?;
    if format == "json" {
        let json = serde_json::to_string_pretty(&goals).map_err(|e| {
            crate::error::TemperError::Vault(format!("json serialization failed: {e}"))
        })?;
        println!("{json}");
        return Ok(());
    }
    if goals.is_empty() {
        output::hint(format!("No goals for context: {context}"));
        return Ok(());
    }
    let task_counts = count_tasks_by_stage(config, context)?;
    let context_title = context.chars().next().unwrap().to_uppercase().to_string() + &context[1..];
    output::header(format!("{context_title} Roadmap"));
    output::plain("\u{2500}".repeat(14));
    // Partition: non-zero seq first (sorted by seq), then zero-seq (maintenance) pinned to bottom
    let (maintenance, regular): (Vec<_>, Vec<_>) = goals.iter().partition(|m| m.seq == 0);
    let ordered: Vec<_> = regular.into_iter().chain(maintenance).collect();
    for g in &ordered {
        let g_counts = task_counts.get(&g.slug);
        let mut stage_parts: Vec<String> = Vec::new();
        for stage in &["backlog", "in-progress", "done"] {
            let count = g_counts.and_then(|c| c.get(*stage)).copied().unwrap_or(0);
            if count > 0 {
                stage_parts.push(format!("{count} {stage}"));
            }
        }
        let counts_str = if stage_parts.is_empty() {
            "empty".to_string()
        } else {
            stage_parts.join(" \u{00b7} ")
        };
        let seq_display = if g.seq == 0 {
            "  \u{00b7}".to_string()
        } else {
            format!("{:>3}", g.seq)
        };
        output::plain(format!(
            " {seq_display}  {:<24} [{:<9}]   {counts_str}",
            g.title, g.status
        ));
    }
    Ok(())
}
