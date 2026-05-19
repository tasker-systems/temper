use std::fs;

use temper_core::vault::Vault;

use crate::actions::types::TaskInfo;
use crate::config::Config;
use crate::error::{Result, TemperError};

/// Load all tasks, optionally filtered by context and/or goal.
pub fn load_tasks(
    config: &Config,
    context: Option<&str>,
    goal_slug: Option<&str>,
) -> Result<Vec<TaskInfo>> {
    let mut tasks = Vec::new();
    let vault_layout = Vault::new(&config.vault_root);
    let dirs: Vec<_> = if let Some(p) = context {
        let owner = config.owner_for_context(p);
        let d = vault_layout.doc_type_dir(&owner, p, "task");
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        // Scan all contexts for task subdirectories
        let mut found = Vec::new();
        for ctx in &config.contexts {
            let owner = config.owner_for_context(ctx);
            let d = vault_layout.doc_type_dir(&owner, ctx, "task");
            if d.is_dir() {
                found.push(d);
            }
        }
        found
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
            let fm = match temper_core::frontmatter::Frontmatter::try_from(content.as_str()) {
                Ok(fm) => fm,
                Err(_) => continue,
            };
            let info: TaskInfo = match serde_yaml::from_value(fm.value().clone()) {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(
                        "skipping {}: frontmatter deserialization failed: {e}",
                        path.display()
                    );
                    continue;
                }
            };
            if let Some(gs) = goal_slug {
                if info.goal.as_deref() != Some(gs) {
                    continue;
                }
            }
            tasks.push(info);
        }
    }
    tasks.sort_by_key(|t| t.seq.unwrap_or(u32::MAX));
    Ok(tasks)
}

/// Get the next seq value for a new task in a goal.
pub fn next_seq(config: &Config, context: &str, goal_slug: &str) -> Result<u32> {
    let tasks = load_tasks(config, Some(context), Some(goal_slug))?;
    let max_seq = tasks.iter().filter_map(|t| t.seq).max().unwrap_or(0);
    Ok(max_seq + 10)
}

/// Find a task by exact slug or unambiguous suffix match.
pub fn find_task(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
) -> Result<Option<TaskInfo>> {
    let all = load_tasks(config, context, None)?;
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
        1 => return Ok(Some(matches[0].clone())),
        n if n > 1 => {
            return Err(TemperError::Vault(format!(
                "ambiguous slug suffix '{slug_or_suffix}', matches: {}",
                matches
                    .iter()
                    .map(|t| t.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
        _ => {}
    }
    // Seq number match
    if let Ok(seq) = slug_or_suffix.parse::<u32>() {
        let seq_matches: Vec<_> = all.iter().filter(|t| t.seq == Some(seq)).collect();
        match seq_matches.len() {
            1 => return Ok(Some(seq_matches[0].clone())),
            n if n > 1 => {
                return Err(TemperError::Vault(format!(
                    "ambiguous seq number '{slug_or_suffix}', matches: {}",
                    seq_matches
                        .iter()
                        .map(|t| t.slug.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
            _ => {}
        }
    }
    Ok(None)
}
