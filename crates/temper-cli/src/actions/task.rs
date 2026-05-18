use std::fs;

use chrono::Local;
use temper_core::vault::Vault;

use crate::actions::types::TaskInfo;
use crate::commands::goal;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;
use crate::vault_backend::per_doctype::{self, DoctypeFields, WriteArgs};

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

/// Create a new task.
///
/// The bare file-write half (template render, frontmatter + body assembly,
/// path computation, hard-error-on-exists, `vault::write_note`) lives in
/// `vault_backend::per_doctype::write_task` so it can be reused by the
/// `VaultBackend` dispatch surface. This wrapper keeps the task-specific
/// concerns:
///   - goal slug resolution + cross-context validation
///   - mode / effort validation
///   - sequence number computation
///   - publish-as-tail-action (`publish_local_write_best_effort`)
///   - discovery event emission
///   - human-readable `output::success`
pub fn create(
    config: &Config,
    context: &str,
    title: &str,
    goal_slug: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    stdin_content: Option<&str>,
) -> Result<String> {
    // Ensure maintenance goal exists if needed
    let gs = match goal_slug {
        Some(gs) => gs.to_string(),
        None => goal::ensure_maintenance(config, context)?,
    };
    // Verify goal exists and context matches
    if let Some(gi) = goal::find_goal(config, &gs, None)? {
        if gi.context != context {
            return Err(TemperError::Vault(format!(
                "goal '{}' belongs to context '{}', not '{context}'",
                gs, gi.context
            )));
        }
    } else if goal_slug.is_some() {
        return Err(TemperError::Vault(format!("goal not found: {gs}")));
    }

    // Validate mode if provided
    if let Some(m) = mode {
        vault::validate_mode(m)?;
    }

    // Validate effort if provided
    if let Some(e) = effort {
        vault::validate_effort(e)?;
    }

    let date = Local::now().format("%Y-%m-%d").to_string();
    let slug_title = vault::slugify(title);
    let slug = format!("{date}-{slug_title}");
    let seq = next_seq(config, context, &gs)?;

    let mode_str = mode.unwrap_or("null");
    let effort_str = effort.unwrap_or("null");

    let owner = config.owner_for_context(context);
    let body = stdin_content.unwrap_or("");
    let result = per_doctype::write_for(WriteArgs {
        doctype: "task",
        title,
        slug: &slug,
        context,
        body,
        open_meta: None,
        vault_root: &config.vault_root,
        owner: &owner,
        config,
        doctype_fields: Some(DoctypeFields::Task {
            goal: &gs,
            mode: mode_str,
            effort: effort_str,
            seq,
        }),
    })?;

    crate::actions::runtime::publish_local_write_best_effort(&config.vault_root, &result.abs_path)?;

    let event = discovery::Event::ResourceCreate {
        ts: Local::now().to_rfc3339(),
        doc_type: "task".to_string(),
        title: title.to_string(),
        path: result.rel_path,
        context: context.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!("Created task: {slug}"));
    Ok(slug)
}

/// Move a task to a new stage and/or goal.
pub fn move_task(
    config: &Config,
    slug_or_suffix: &str,
    stage: Option<&str>,
    new_goal: Option<&str>,
    context: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
) -> Result<()> {
    let task = find_task(config, slug_or_suffix, context)?
        .ok_or_else(|| TemperError::Vault(format!("task not found: {slug_or_suffix}")))?;

    if let Some(s) = stage {
        vault::validate_stage(s)?;
    }

    if let Some(m) = mode {
        vault::validate_mode(m)?;
    }

    if let Some(e) = effort {
        vault::validate_effort(e)?;
    }

    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(&task.context);
    let path = vault_layout.doc_file(&owner, &task.context, "task", &task.slug);
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&path)?;

    let from_stage = task.stage.clone();
    let to_stage = stage.unwrap_or(&from_stage);

    if let Some(s) = stage {
        fm.set_managed_field("temper-stage", serde_json::Value::String(s.to_string()));
    }

    if let Some(g) = new_goal {
        // Validate goal exists and context matches
        let goal_info = goal::find_goal(config, g, None)?
            .ok_or_else(|| TemperError::Vault(format!("goal not found: {g}")))?;
        if goal_info.context != task.context {
            return Err(TemperError::Vault(format!(
                "goal '{}' belongs to context '{}', not '{}'",
                g, goal_info.context, task.context
            )));
        }
        fm.set_managed_field("temper-goal", serde_json::Value::String(g.to_string()));
        // Assign new seq at end of target goal
        let new_seq = next_seq(config, &task.context, g)?;
        fm.set_managed_field("temper-seq", serde_json::Value::from(new_seq));
    }

    if let Some(m) = mode {
        fm.set_managed_field("temper-mode", serde_json::Value::String(m.to_string()));
    }

    if let Some(e) = effort {
        fm.set_managed_field("temper-effort", serde_json::Value::String(e.to_string()));
    }

    let datetime = Local::now().to_rfc3339();
    fm.set_managed_field(
        "temper-updated",
        serde_json::Value::String(datetime.clone()),
    );
    fm.write_to(&path)?;

    let event = discovery::Event::ResourceUpdate {
        ts: datetime,
        doc_type: "task".to_string(),
        slug: task.slug.clone(),
        context: task.context,
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!(
        "Moved task {}: {from_stage} → {to_stage}",
        task.slug
    ));
    Ok(())
}

/// Mark a task as done with branch and PR info.
pub fn done(
    config: &Config,
    slug_or_suffix: &str,
    branch: Option<&str>,
    pr: Option<&str>,
    context: Option<&str>,
) -> Result<()> {
    let task = find_task(config, slug_or_suffix, context)?
        .ok_or_else(|| TemperError::Vault(format!("task not found: {slug_or_suffix}")))?;

    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(&task.context);
    let path = vault_layout.doc_file(&owner, &task.context, "task", &task.slug);
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&path)?;

    let datetime = Local::now().to_rfc3339();
    fm.set_managed_field(
        "temper-stage",
        serde_json::Value::String("done".to_string()),
    );
    fm.set_managed_field(
        "temper-updated",
        serde_json::Value::String(datetime.clone()),
    );
    if let Some(b) = branch {
        fm.set_managed_field("temper-branch", serde_json::Value::String(b.to_string()));
    }
    if let Some(p) = pr {
        fm.set_managed_field("temper-pr", serde_json::Value::String(p.to_string()));
    }
    fm.write_to(&path)?;

    let event = discovery::Event::ResourceUpdate {
        ts: datetime,
        doc_type: "task".to_string(),
        slug: task.slug.clone(),
        context: task.context,
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }
    output::success(format!("Completed task: {}", task.slug));
    Ok(())
}
