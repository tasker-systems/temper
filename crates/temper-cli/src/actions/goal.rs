use std::fs;

use askama::Template;
use chrono::Local;

use crate::actions::types::GoalInfo;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::templates::GoalTemplate;
use crate::vault;

/// Load all goals, optionally filtered by context, sorted by seq.
pub fn load_goals(config: &Config, context: Option<&str>) -> Result<Vec<GoalInfo>> {
    let mut goals = Vec::new();
    let dirs: Vec<_> = if let Some(p) = context {
        let d = config.doc_type_dir(p, "goal");
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        // Scan all contexts for goal subdirectories
        let mut found = Vec::new();
        for ctx in &config.contexts {
            let d = config.doc_type_dir(ctx, "goal");
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
            let fm = match vault::parse_frontmatter(&content) {
                Some(fm) => fm,
                None => continue,
            };
            let info: GoalInfo = match serde_yaml::from_value(fm) {
                Ok(i) => i,
                Err(_) => continue,
            };
            goals.push(info);
        }
    }
    goals.sort_by_key(|m| m.seq);
    Ok(goals)
}

/// Get the next seq value for a new goal in a context (max seq + 10, minimum 10).
pub fn next_seq(config: &Config, context: &str) -> Result<u32> {
    let goals = load_goals(config, Some(context))?;
    let max_seq = goals.iter().map(|m| m.seq).max().unwrap_or(0);
    Ok(max_seq + 10)
}

/// Find a goal by slug, optionally scoped to a context.
pub fn find_goal(config: &Config, slug: &str, context: Option<&str>) -> Result<Option<GoalInfo>> {
    let goals = load_goals(config, context)?;
    Ok(goals.into_iter().find(|m| m.slug == slug))
}

/// Ensure the maintenance goal exists for a context, creating it if missing.
pub fn ensure_maintenance(config: &Config, context: &str) -> Result<String> {
    let slug = format!("{context}-maintenance");
    let dir = config.doc_type_dir(context, "goal");
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Ok(slug);
    }
    let id = crate::ids::generate_id();
    let date = Local::now().format("%Y-%m-%d").to_string();
    let tmpl = GoalTemplate {
        id: &id,
        title: "Maintenance",
        slug: &slug,
        context,
        seq: "0",
        date: &date,
    };
    let content = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    let event = discovery::Event::ResourceCreate {
        ts: Local::now().to_rfc3339(),
        doc_type: "goal".to_string(),
        title: "Maintenance".to_string(),
        path: format!("{context}/goal/{slug}.md"),
        context: context.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    Ok(slug)
}

/// Create a new goal. Returns the slug of the created goal.
pub fn create(config: &Config, context: &str, title: &str, slug: Option<&str>) -> Result<String> {
    let slug = match slug {
        Some(s) => s.to_string(),
        None => vault::slugify(title),
    };
    let dir = config.doc_type_dir(context, "goal");
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Err(TemperError::Vault(format!("goal already exists: {slug}")));
    }
    let seq = next_seq(config, context)?;
    let seq_str = seq.to_string();
    let id = crate::ids::generate_id();
    let date = Local::now().format("%Y-%m-%d").to_string();
    let tmpl = GoalTemplate {
        id: &id,
        title,
        slug: &slug,
        context,
        seq: &seq_str,
        date: &date,
    };
    let content = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    let event = discovery::Event::ResourceCreate {
        ts: Local::now().to_rfc3339(),
        doc_type: "goal".to_string(),
        title: title.to_string(),
        path: format!("{context}/goal/{slug}.md"),
        context: context.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    Ok(slug)
}

/// Update a goal's status.
pub fn update(config: &Config, slug: &str, status: &str, context: Option<&str>) -> Result<()> {
    let valid_statuses = ["active", "completed", "paused", "cancelled"];
    if !valid_statuses.contains(&status) {
        return Err(TemperError::Vault(format!(
            "invalid status: {status}. Must be one of: {}",
            valid_statuses.join(", ")
        )));
    }
    let info = find_goal(config, slug, context)?
        .ok_or_else(|| TemperError::Vault(format!("goal not found: {slug}")))?;
    let path = config
        .doc_type_dir(&info.context, "goal")
        .join(format!("{slug}.md"));
    if !path.exists() {
        return Err(TemperError::Vault(format!("goal not found: {slug}")));
    }
    let content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let updated = vault::set_frontmatter_field(&content, "temper-status", status);
    fs::write(&path, updated).map_err(|e| TemperError::Vault(e.to_string()))?;
    let event = discovery::Event::ResourceUpdate {
        ts: Local::now().to_rfc3339(),
        doc_type: "goal".to_string(),
        slug: slug.to_string(),
        context: info.context,
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    Ok(())
}

/// Count tasks per goal and stage by scanning task files directly.
pub fn count_tasks_by_stage(
    config: &Config,
    context: &str,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, usize>>> {
    let vault_layout = temper_core::vault::Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let dir = vault_layout.doc_type_dir(&owner, context, "task");
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
        let g = fm
            .get("temper-goal")
            .or_else(|| fm.get("goal"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let stage = fm
            .get("temper-stage")
            .or_else(|| fm.get("stage"))
            .and_then(|v| v.as_str())
            .unwrap_or("backlog");
        *counts
            .entry(g.to_string())
            .or_default()
            .entry(stage.to_string())
            .or_default() += 1;
    }
    Ok(counts)
}
