use crate::config::{self, Config};
use crate::error::Result;
use crate::output;

/// Add a context to sync.subscriptions.contexts in the global config.
pub fn add(name: &str) -> Result<()> {
    let config_path = config::global_config_path();

    config::safe_write(&config_path, |content| {
        // Check if the context already exists
        if content.contains(&format!("\"{name}\"")) {
            return content;
        }
        // Find the contexts line and append
        let mut result = String::new();
        for line in content.lines() {
            if line.trim_start().starts_with("contexts") && line.contains('[') {
                // Parse existing array and add new context
                if let Some(bracket_start) = line.find('[') {
                    if let Some(bracket_end) = line.find(']') {
                        let existing = &line[bracket_start + 1..bracket_end];
                        let trimmed = existing.trim();
                        let new_line = if trimmed.is_empty() {
                            format!("{}[\"{name}\"]", &line[..bracket_start])
                        } else {
                            format!("{}[{}, \"{name}\"]", &line[..bracket_start], trimmed)
                        };
                        result.push_str(&new_line);
                        result.push('\n');
                        continue;
                    }
                }
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    })?;

    output::success(format!("Added context '{name}'"));
    Ok(())
}

/// Remove a context from sync.subscriptions.contexts in the global config.
pub fn remove(name: &str) -> Result<()> {
    let config_path = config::global_config_path();

    config::safe_write(&config_path, |content| {
        let mut result = String::new();
        for line in content.lines() {
            if line.trim_start().starts_with("contexts") && line.contains('[') {
                if let Some(bracket_start) = line.find('[') {
                    if let Some(bracket_end) = line.find(']') {
                        let existing = &line[bracket_start + 1..bracket_end];
                        // Parse items, filter out the one to remove
                        let items: Vec<&str> = existing
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .filter(|s| {
                                let unquoted = s.trim_matches('"');
                                unquoted != name
                            })
                            .collect();
                        let new_line = format!("{}[{}]", &line[..bracket_start], items.join(", "));
                        result.push_str(&new_line);
                        result.push('\n');
                        continue;
                    }
                }
            }
            result.push_str(line);
            result.push('\n');
        }
        result
    })?;

    output::success(format!("Removed context '{name}'"));
    Ok(())
}

/// List configured contexts.
pub fn list(config: &Config) -> Result<()> {
    if config.contexts.is_empty() {
        output::hint("No contexts configured.");
        return Ok(());
    }

    let mut names = config.contexts.clone();
    names.sort();

    output::plain(format!("{:<30} CONTEXT", "NAME"));
    output::dim("-".repeat(40));
    for name in &names {
        output::plain(format!("{:<30} {name}", name));
    }

    Ok(())
}
