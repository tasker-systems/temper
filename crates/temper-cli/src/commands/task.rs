use crate::config::Config;
use crate::error::{Result, TemperError};

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
    let path = config
        .doc_type_dir(&task.context, "task")
        .join(format!("{}.md", task.slug));
    let content = std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    print!("{content}");
    Ok(())
}
