use crate::config::Config;
use crate::error::Result;
use crate::output;

// Re-export data types and pure functions from the actions layer
pub use crate::actions::goal::{ensure_maintenance, find_goal, load_goals, next_seq};
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
            .and_then(|m| m.seq);
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
