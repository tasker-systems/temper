use std::fs;

use askama::Template;
use chrono::Local;
use temper_core::vault::Vault;

use crate::actions::types::GoalInfo;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::templates::GoalTemplate;
use crate::vault;

/// Load all goals, optionally filtered by context, sorted by seq.
pub fn load_goals(config: &Config, context: Option<&str>) -> Result<Vec<GoalInfo>> {
    let mut goals = Vec::new();
    let vault_layout = Vault::new(&config.vault_root);
    let dirs: Vec<_> = if let Some(p) = context {
        let owner = config.owner_for_context(p);
        let d = vault_layout.doc_type_dir(&owner, p, "goal");
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        // Scan all contexts for goal subdirectories
        let mut found = Vec::new();
        for ctx in &config.contexts {
            let owner = config.owner_for_context(ctx);
            let d = vault_layout.doc_type_dir(&owner, ctx, "goal");
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
            let info: GoalInfo = match serde_yaml::from_value(fm.value().clone()) {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(
                        "skipping {}: frontmatter deserialization failed: {e}",
                        path.display()
                    );
                    continue;
                }
            };
            goals.push(info);
        }
    }
    goals.sort_by_key(|m| m.seq.unwrap_or(u32::MAX));
    Ok(goals)
}

/// Get the next seq value for a new goal in a context (max seq + 10, minimum 10).
pub fn next_seq(config: &Config, context: &str) -> Result<u32> {
    let goals = load_goals(config, Some(context))?;
    let max_seq = goals.iter().filter_map(|m| m.seq).max().unwrap_or(0);
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
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let dir = vault_layout.doc_type_dir(&owner, context, "goal");
    let path = vault_layout.doc_file(&owner, context, "goal", &slug);
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
        path: vault_layout.rel_path(&owner, context, "goal", &slug),
        context: context.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    Ok(slug)
}
