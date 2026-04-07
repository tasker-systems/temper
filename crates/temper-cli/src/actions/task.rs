use std::fs;

use askama::Template;
use chrono::Local;

use crate::actions::types::TaskInfo;
use crate::commands::goal;
use crate::config::Config;
use crate::discovery;
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::TaskTemplate;
use crate::vault;

/// Load all tasks, optionally filtered by context and/or goal.
pub fn load_tasks(
    config: &Config,
    context: Option<&str>,
    goal_slug: Option<&str>,
) -> Result<Vec<TaskInfo>> {
    let mut tasks = Vec::new();
    let dirs: Vec<_> = if let Some(p) = context {
        let d = config.doc_type_dir(p, "task");
        if d.is_dir() {
            vec![d]
        } else {
            vec![]
        }
    } else {
        // Scan all contexts for task subdirectories
        let mut found = Vec::new();
        for ctx in &config.contexts {
            let d = config.doc_type_dir(ctx, "task");
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
            let info: TaskInfo = match serde_yaml::from_value(fm) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if let Some(gs) = goal_slug {
                if info.goal != gs {
                    continue;
                }
            }
            tasks.push(info);
        }
    }
    tasks.sort_by_key(|t| t.seq);
    Ok(tasks)
}

/// Get the next seq value for a new task in a goal.
pub fn next_seq(config: &Config, context: &str, goal_slug: &str) -> Result<u32> {
    let tasks = load_tasks(config, Some(context), Some(goal_slug))?;
    let max_seq = tasks.iter().map(|t| t.seq).max().unwrap_or(0);
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
        let seq_matches: Vec<_> = all.iter().filter(|t| t.seq == seq).collect();
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
pub fn create(
    config: &Config,
    context: &str,
    title: &str,
    goal_slug: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
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
    let datetime = Local::now().to_rfc3339();
    let seq = next_seq(config, context, &gs)?;
    let seq_str = seq.to_string();
    let id = crate::ids::generate_id();

    let mode_str = mode.unwrap_or("null");
    let effort_str = effort.unwrap_or("null");
    let tmpl = TaskTemplate {
        id: &id,
        title,
        slug: &slug,
        context,
        goal: &gs,
        mode: mode_str,
        effort: effort_str,
        seq: &seq_str,
        datetime: &datetime,
    };
    let mut content = tmpl
        .render()
        .map_err(|e| TemperError::Vault(format!("template error: {e}")))?;

    if let Some(stdin_content) = vault::read_stdin_if_piped() {
        content.push_str(&stdin_content);
        content.push('\n');
    }

    let dir = config.doc_type_dir(context, "task");
    fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    let path = dir.join(format!("{slug}.md"));
    vault::write_note(&path, &content)?;

    let event = discovery::Event::ResourceCreate {
        ts: datetime,
        doc_type: "task".to_string(),
        title: title.to_string(),
        path: format!("{context}/task/{slug}.md"),
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

    let path = config
        .doc_type_dir(&task.context, "task")
        .join(format!("{}.md", task.slug));
    let mut content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;

    let from_stage = task.stage.clone();
    let to_stage = stage.unwrap_or(&from_stage);

    if let Some(s) = stage {
        content = vault::set_frontmatter_field(&content, "temper-stage", s);
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
        content = vault::set_frontmatter_field(&content, "temper-goal", g);
        // Assign new seq at end of target goal
        let new_seq = next_seq(config, &task.context, g)?;
        content = vault::set_frontmatter_field(&content, "temper-seq", &new_seq.to_string());
    }

    if let Some(m) = mode {
        content = vault::set_frontmatter_field(&content, "temper-mode", m);
    }

    if let Some(e) = effort {
        content = vault::set_frontmatter_field(&content, "temper-effort", e);
    }

    let datetime = Local::now().to_rfc3339();
    content = vault::set_frontmatter_field(&content, "temper-updated", &datetime);
    fs::write(&path, &content).map_err(|e| TemperError::Vault(e.to_string()))?;

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

    let path = config
        .doc_type_dir(&task.context, "task")
        .join(format!("{}.md", task.slug));
    let mut content = fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;

    let datetime = Local::now().to_rfc3339();
    content = vault::set_frontmatter_field(&content, "temper-stage", "done");
    content = vault::set_frontmatter_field(&content, "temper-updated", &datetime);
    if let Some(b) = branch {
        content = vault::set_frontmatter_field(&content, "temper-branch", b);
    }
    if let Some(p) = pr {
        content = vault::set_frontmatter_field(&content, "temper-pr", p);
    }
    fs::write(&path, &content).map_err(|e| TemperError::Vault(e.to_string()))?;

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
