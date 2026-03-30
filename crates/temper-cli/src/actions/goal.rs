use std::fs;

use chrono::Local;

use crate::actions::types::GoalInfo;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::vault;

/// Load all goals, optionally filtered by context, sorted by seq.
pub fn load_goals(config: &Config, context: Option<&str>) -> Result<Vec<GoalInfo>> {
    let base = &config.goals_dir;
    if !base.is_dir() {
        return Ok(vec![]);
    }
    let mut goals = Vec::new();
    let dirs: Vec<_> = if let Some(p) = context {
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
    let dir = config.goals_dir.join(context);
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
        ("context", context),
        ("seq", "0"),
        ("id", id.as_str()),
    ];
    let content = vault::render_template_with_vars(
        &config.vault_root,
        templates_dir,
        "goal",
        "Maintenance",
        &vars,
    )?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    let event = discovery::Event::GoalCreate {
        ts: Local::now().to_rfc3339(),
        context: context.to_string(),
        goal: slug.clone(),
        title: "Maintenance".to_string(),
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
    let dir = config.goals_dir.join(context);
    let path = dir.join(format!("{slug}.md"));
    if path.exists() {
        return Err(TemperError::Vault(format!("goal already exists: {slug}")));
    }
    let seq = next_seq(config, context)?;
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
        ("context", context),
        ("seq", seq_str.as_str()),
        ("id", id.as_str()),
    ];
    let content =
        vault::render_template_with_vars(&config.vault_root, templates_dir, "goal", title, &vars)?;
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    vault::write_note(&path, &content)?;
    let event = discovery::Event::GoalCreate {
        ts: Local::now().to_rfc3339(),
        context: context.to_string(),
        goal: slug.clone(),
        title: title.to_string(),
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
        .goals_dir
        .join(&info.context)
        .join(format!("{slug}.md"));
    if !path.exists() {
        return Err(TemperError::Vault(format!("goal not found: {slug}")));
    }
    let content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let updated = vault::set_frontmatter_field(&content, "status", status);
    fs::write(&path, updated).map_err(|e| TemperError::Vault(e.to_string()))?;
    let event = discovery::Event::GoalUpdate {
        ts: Local::now().to_rfc3339(),
        context: info.context,
        goal: slug.to_string(),
        status: status.to_string(),
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
    let dir = config.tasks_dir.join(context);
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
        let g = fm.get("goal").and_then(|v| v.as_str()).unwrap_or("unknown");
        let stage = fm
            .get("stage")
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
