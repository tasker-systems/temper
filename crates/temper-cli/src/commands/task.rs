use temper_core::vault::Vault;

use crate::config::Config;
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;

// Re-export data types and functions from the actions layer
pub use crate::actions::task::{create, done, find_task, load_tasks, move_task, next_seq};
pub use crate::actions::types::TaskInfo;

/// Show a single task's raw markdown content.
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    let task = find_task(config, slug_or_suffix, context)?
        .ok_or_else(|| TemperError::Vault(format!("task not found: {slug_or_suffix}")))?;
    if format == "json" {
        let json = serde_json::to_string_pretty(&task)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(&task.context);
    let path = vault_layout.doc_file(&owner, &task.context, "task", &task.slug);
    let content = std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    print!("{content}");
    Ok(())
}

/// List tasks grouped by goal.
pub fn list(
    config: &Config,
    context: Option<&str>,
    goal_slug: Option<&str>,
    stage: Option<&str>,
    format: &str,
) -> Result<()> {
    if let Some(s) = stage {
        vault::validate_stage(s)?;
    }
    let mut tasks = load_tasks(config, context, goal_slug)?;
    if let Some(s) = stage {
        tasks.retain(|t| t.stage == s);
    }
    if format == "json" {
        let json = serde_json::to_string_pretty(&tasks)
            .map_err(|e| TemperError::Vault(format!("json serialization failed: {e}")))?;
        println!("{json}");
        return Ok(());
    }
    if tasks.is_empty() {
        output::hint("No tasks found.");
        return Ok(());
    }
    // Group by goal
    let mut by_goal: std::collections::BTreeMap<String, Vec<&TaskInfo>> =
        std::collections::BTreeMap::new();
    for task in &tasks {
        by_goal.entry(task.goal.clone()).or_default().push(task);
    }
    for (g, tix) in &by_goal {
        output::blank();
        output::header(format!("## {g}"));
        for t in tix {
            output::plain(format!(
                "  {:>3}  [{:<10}]  {} ({})",
                t.seq, t.stage, t.title, t.context
            ));
        }
    }
    Ok(())
}
