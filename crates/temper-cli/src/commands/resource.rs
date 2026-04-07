use std::path::PathBuf;

use askama::Template;
use chrono::Local;
use serde::Serialize;
use temper_core::schema;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::{Result, TemperError};
use crate::output;
use crate::templates::{ConceptTemplate, DecisionTemplate};
use crate::vault;

const VALID_DOC_TYPES: &[&str] = &["task", "goal", "session", "research", "concept", "decision"];

fn validate_doc_type(doc_type: &str) -> Result<()> {
    if !VALID_DOC_TYPES.contains(&doc_type) {
        return Err(TemperError::Vault(format!(
            "invalid resource type: {doc_type}. Must be one of: {}",
            VALID_DOC_TYPES.join(", ")
        )));
    }
    Ok(())
}

/// Require a context, returning an error if none specified.
fn require_context(config: &Config, context: Option<&str>) -> Result<String> {
    match context {
        Some(ctx) => Ok(super::resolve_context_with_fallback(config, ctx).into_owned()),
        None => Err(TemperError::Project(
            "no context specified — use --context <name>".into(),
        )),
    }
}

/// Create a new resource.
pub fn create(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    goal: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    slug: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_doc_type(doc_type)?;

    let ctx = require_context(config, context)?;

    match doc_type {
        "task" => {
            let created_slug =
                crate::actions::task::create(config, &ctx, title, goal, mode, effort)?;
            if format == "json" {
                let json = serde_json::json!({
                    "type": "task",
                    "slug": created_slug,
                    "title": title,
                    "context": &*ctx,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or_default()
                );
            }
            Ok(())
        }
        "goal" => {
            crate::commands::goal::create(config, &ctx, title, slug, format)?;
            Ok(())
        }
        "session" => {
            let stdin_content = vault::read_stdin_if_piped();
            crate::commands::session::save(
                config,
                Some(title),
                Some(ctx.as_str()),
                stdin_content.as_deref(),
                None, // task
                None, // state
                format,
            )
        }
        "research" => {
            let stdin_content = vault::read_stdin_if_piped();
            crate::commands::research::save(
                config,
                title,
                Some(ctx.as_str()),
                stdin_content.as_deref(),
                format,
            )
        }
        "concept" | "decision" => {
            create_simple_resource(config, doc_type, title, &ctx, slug, format)
        }
        _ => Err(TemperError::Vault(format!(
            "unsupported resource type for create: {doc_type}"
        ))),
    }
}

/// Create a concept or decision resource using Askama templates.
fn create_simple_resource(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: &str,
    slug_override: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let id = crate::ids::generate_id();
    let slug = match slug_override {
        Some(s) => s.to_string(),
        None => format!("{today}-{}", vault::slugify(title)),
    };

    let content = match doc_type {
        "concept" => {
            let tmpl = ConceptTemplate {
                id: &id,
                title,
                date: &today,
                project: context,
                slug: &slug,
            };
            tmpl.render()
                .map_err(|e| TemperError::Vault(format!("template error: {e}")))?
        }
        "decision" => {
            let tmpl = DecisionTemplate {
                id: &id,
                title,
                date: &today,
                project: context,
                slug: &slug,
            };
            tmpl.render()
                .map_err(|e| TemperError::Vault(format!("template error: {e}")))?
        }
        _ => unreachable!(),
    };

    // Handle stdin body replacement
    let content = if let Some(body) = vault::read_stdin_if_piped() {
        vault::replace_body(&content, &body)
    } else {
        content
    };

    let dir = config.doc_type_dir(context, doc_type);
    let path = dir.join(format!("{slug}.md"));

    if path.exists() {
        return Err(TemperError::Vault(format!(
            "{doc_type} already exists: {slug}"
        )));
    }

    vault::write_note(&path, &content)?;

    let relative = path.strip_prefix(&config.vault_root).unwrap_or(&path);
    let relative_str = relative.to_string_lossy();

    if format == "json" {
        #[derive(Serialize)]
        struct ResourceCreated<'a> {
            doc_type: &'a str,
            title: &'a str,
            slug: &'a str,
            context: &'a str,
            path: &'a str,
            date: &'a str,
            id: &'a str,
        }
        let info = ResourceCreated {
            doc_type,
            title,
            slug: &slug,
            context,
            path: &relative_str,
            date: &today,
            id: &id,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
    } else {
        output::success(format!("Created: {relative_str}"));
    }

    // Emit discovery event
    let ts = Local::now().to_rfc3339();
    let event = Event::ResourceCreate {
        ts,
        doc_type: doc_type.to_string(),
        title: title.to_string(),
        path: relative_str.to_string(),
        context: context.to_string(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    Ok(())
}

/// List resources of a given type.
pub fn list(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
    limit: Option<usize>,
    stage: Option<&str>,
    goal: Option<&str>,
    status: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_doc_type(doc_type)?;

    match doc_type {
        "task" => {
            let ctx = context.map(|c| super::resolve_context_with_fallback(config, c).into_owned());
            crate::commands::task::list(config, ctx.as_deref(), goal, stage, format)
        }
        "goal" => {
            let ctx = require_context(config, context)?;
            if status.is_some() {
                output::hint("--status filter is not yet supported for goals; listing all.");
            }
            crate::commands::goal::list(config, &ctx, format)
        }
        "session" => crate::commands::session::list(config, context, limit, format),
        "research" | "concept" | "decision" => {
            list_simple_resources(config, doc_type, context, limit, format)
        }
        _ => Err(TemperError::Vault(format!(
            "unsupported resource type for list: {doc_type}"
        ))),
    }
}

/// List simple resources (research, concept, decision) by scanning the doc_type directory.
fn list_simple_resources(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
    limit: Option<usize>,
    format: &str,
) -> Result<()> {
    let mut entries: Vec<SimpleResourceEntry> = Vec::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    for ctx in &contexts_to_scan {
        let dir = config.doc_type_dir(ctx, doc_type);
        if dir.is_dir() {
            collect_simple_resources(&dir, ctx, &mut entries)?;
        }
    }

    // Sort by date descending (most recent first)
    entries.sort_by(|a, b| b.date.cmp(&a.date));
    entries.truncate(limit.unwrap_or(20));

    if format == "json" {
        let json = serde_json::to_string_pretty(&entries).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    if entries.is_empty() {
        output::hint(format!("No {doc_type} resources found."));
        return Ok(());
    }

    output::plain(format!("{:<12} {:<20} Title", "Date", "Context"));
    output::dim("-".repeat(60));
    for entry in &entries {
        output::plain(format!(
            "{:<12} {:<20} {}",
            entry.date, entry.context, entry.title
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct SimpleResourceEntry {
    date: String,
    context: String,
    title: String,
    slug: String,
}

fn collect_simple_resources(
    dir: &std::path::Path,
    context: &str,
    entries: &mut Vec<SimpleResourceEntry>,
) -> Result<()> {
    for file_entry in std::fs::read_dir(dir)? {
        let file_entry = file_entry?;
        let path = file_entry.path();
        if path.extension().is_some_and(|e| e == "md") {
            if let Some(entry) = parse_simple_resource(&path, context) {
                entries.push(entry);
            }
        }
    }
    Ok(())
}

fn parse_simple_resource(path: &std::path::Path, context: &str) -> Option<SimpleResourceEntry> {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let content = std::fs::read_to_string(path).ok()?;
    let fm = vault::parse_frontmatter(&content);

    let title = fm
        .as_ref()
        .and_then(|v| v.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or(&stem)
        .to_string();

    let date = fm
        .as_ref()
        .and_then(|v| v.get("date"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| extract_date_prefix(&stem))
        .unwrap_or_else(|| "unknown".to_string());

    let slug = fm
        .as_ref()
        .and_then(|v| v.get("slug"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| stem.clone());

    Some(SimpleResourceEntry {
        date,
        context: context.to_string(),
        title,
        slug,
    })
}

/// Extract a YYYY-MM-DD date prefix from a filename stem.
fn extract_date_prefix(stem: &str) -> Option<String> {
    if stem.len() >= 10 {
        let candidate = &stem[..10];
        let bytes = candidate.as_bytes();
        if bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[..4].iter().all(|b| b.is_ascii_digit())
            && bytes[5..7].iter().all(|b| b.is_ascii_digit())
            && bytes[8..10].iter().all(|b| b.is_ascii_digit())
        {
            return Some(candidate.to_string());
        }
    }
    None
}

/// Show a resource's content.
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    validate_doc_type(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }
}

/// Show a generic resource (goal, research, concept, decision) by finding and
/// printing its file content.
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    let (path, ctx) = find_resource_file(config, doc_type, slug, context)?;

    let content = std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;

    if format == "json" {
        let fm = vault::parse_frontmatter(&content);
        let title = fm
            .as_ref()
            .and_then(|v| v.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or(slug);
        let relative = path.strip_prefix(&config.vault_root).unwrap_or(&path);

        #[derive(Serialize)]
        struct ResourceShow<'a> {
            doc_type: &'a str,
            slug: &'a str,
            title: &'a str,
            context: &'a str,
            path: String,
            content: String,
        }
        let info = ResourceShow {
            doc_type,
            slug,
            title,
            context: &ctx,
            path: relative.to_string_lossy().to_string(),
            content,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    print!("{content}");
    Ok(())
}

/// Find a resource file by slug, searching across contexts if none specified.
///
/// Matches by exact stem, slug portion after date prefix (e.g.
/// `2026-04-06-my-slug` matches `my-slug`), or contains needle.
fn find_resource_file(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
) -> Result<(PathBuf, String)> {
    let contexts_to_scan: Vec<String> = if let Some(ctx) = context {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    let needle = vault::slugify(slug);
    let mut matches: Vec<(PathBuf, String)> = Vec::new();

    for ctx in &contexts_to_scan {
        let dir = config.doc_type_dir(ctx, doc_type);
        if !dir.is_dir() {
            continue;
        }
        for file_entry in std::fs::read_dir(&dir)? {
            let file_entry = file_entry?;
            let path = file_entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Extract slug portion after date prefix (YYYY-MM-DD-)
            let slug_portion = if stem.len() > 11
                && stem.as_bytes()[4] == b'-'
                && stem.as_bytes()[7] == b'-'
                && stem.as_bytes()[10] == b'-'
            {
                &stem[11..]
            } else {
                &stem
            };

            // Match: exact stem, exact slug portion, or contains needle
            if stem == needle || slug_portion == needle || stem.contains(&needle) {
                matches.push((path, ctx.clone()));
            }
        }
    }

    if matches.is_empty() {
        return Err(TemperError::Vault(format!("{doc_type} not found: {slug}")));
    }

    // Sort by path descending (most recent date-prefixed file first)
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    let (path, ctx) = matches.into_iter().next().unwrap();
    Ok((path, ctx))
}

/// Update a resource's frontmatter fields.
#[allow(clippy::too_many_arguments)]
pub fn update(
    config: &Config,
    slug: &str,
    doc_type: Option<&str>,
    type_from: Option<&str>,
    type_to: Option<&str>,
    context: Option<&str>,
    context_to: Option<&str>,
    title: Option<&str>,
    tags: &[String],
    aliases: &[String],
    relates_to: &[String],
    references: &[String],
    depends_on: &[String],
    stage: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    goal: Option<&str>,
    seq: Option<i64>,
    branch: Option<&str>,
    pr: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    // Resolve current type from --type or --type-from (one is required)
    let current_type = doc_type
        .or(type_from)
        .ok_or_else(|| TemperError::Vault("--type or --type-from is required".into()))?;
    validate_doc_type(current_type)?;

    if let Some(tt) = type_to {
        validate_doc_type(tt)?;
    }

    // Find the resource file
    let (path, ctx) = find_resource_file(config, current_type, slug, context)?;

    // Load updatable fields from schema for validation
    let schema_fields = schema::updatable_fields(current_type)?;

    // Build list of scalar field updates: (frontmatter_key, value)
    let scalar_updates: Vec<(&str, String)> = [
        ("title", title.map(String::from)),
        ("temper-stage", stage.map(String::from)),
        ("temper-mode", mode.map(String::from)),
        ("temper-effort", effort.map(String::from)),
        ("temper-goal", goal.map(String::from)),
        ("temper-branch", branch.map(String::from)),
        ("temper-pr", pr.map(String::from)),
        ("temper-status", status.map(String::from)),
        ("temper-seq", seq.map(|s| s.to_string())),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|val| (k, val)))
    .collect();

    // Base fields valid on all types (from base.schema.json)
    const BASE_FIELDS: &[&str] = &["title"];

    // Validate scalar fields against schema
    for (field_name, value) in &scalar_updates {
        if BASE_FIELDS.contains(field_name) {
            continue; // Always valid
        }
        match schema_fields.iter().find(|(n, _)| n == field_name) {
            Some((_name, schema_prop)) => {
                if let Some(err) = schema::validate_field_value(field_name, value, schema_prop) {
                    return Err(TemperError::Project(err));
                }
            }
            None => {
                let flag = field_name.strip_prefix("temper-").unwrap_or(field_name);
                return Err(TemperError::Project(format!(
                    "--{flag} is not valid for type '{current_type}'"
                )));
            }
        }
    }

    // Read and modify content
    let mut content =
        std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;

    // Apply scalar field updates
    for (field_name, value) in &scalar_updates {
        content = vault::set_frontmatter_field(&content, field_name, value);
    }

    // Apply array field appends
    let array_updates: Vec<(&str, &[String])> = vec![
        ("tags", tags),
        ("aliases", aliases),
        ("relates_to", relates_to),
        ("references", references),
        ("depends_on", depends_on),
    ];
    for (field_name, values) in &array_updates {
        for value in *values {
            content = append_frontmatter_array(&content, field_name, value);
        }
    }

    // Handle --context-to (move file to new context dir, update temper-context)
    let final_ctx;
    let mut final_path = path.clone();
    if let Some(new_ctx) = context_to {
        let new_dir = config.doc_type_dir(new_ctx, type_to.unwrap_or(current_type));
        std::fs::create_dir_all(&new_dir)?;
        let filename = path
            .file_name()
            .ok_or_else(|| TemperError::Vault("cannot determine filename".into()))?;
        let new_path = new_dir.join(filename);
        content = vault::set_frontmatter_field(&content, "temper-context", new_ctx);
        final_path = new_path;
        final_ctx = new_ctx.to_string();
    } else {
        final_ctx = ctx.clone();
    }

    // Handle --type-to (move file to new type dir, update temper-type)
    if let Some(new_type) = type_to {
        let target_ctx = context_to.unwrap_or(&final_ctx);
        let new_dir = config.doc_type_dir(target_ctx, new_type);
        std::fs::create_dir_all(&new_dir)?;
        let filename = final_path
            .file_name()
            .ok_or_else(|| TemperError::Vault("cannot determine filename".into()))?;
        let new_path = new_dir.join(filename);
        content = vault::set_frontmatter_field(&content, "temper-type", new_type);
        final_path = new_path;
    }

    // Update temper-updated timestamp
    let datetime = Local::now().to_rfc3339();
    content = vault::set_frontmatter_field(&content, "temper-updated", &datetime);

    // Write updated content
    std::fs::write(&final_path, &content).map_err(|e| TemperError::Vault(e.to_string()))?;

    // If file was moved, remove old file
    if final_path != path && path.exists() {
        std::fs::remove_file(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    }

    // Determine slug for output
    let final_slug = final_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Emit ResourceUpdate discovery event
    let event = Event::ResourceUpdate {
        ts: datetime,
        doc_type: type_to.unwrap_or(current_type).to_string(),
        slug: final_slug.clone(),
        context: final_ctx.clone(),
    };
    if let Err(e) = discovery::append_event(&config.state_dir, &event) {
        tracing::warn!("Failed to append discovery event: {e}");
    }

    let relative = final_path
        .strip_prefix(&config.vault_root)
        .unwrap_or(&final_path);
    output::success(format!("Updated: {}", relative.display()));
    Ok(())
}

/// Append a value to a YAML array field in frontmatter.
///
/// If the field exists, appends `\n  - value` after the field marker.
/// If the field doesn't exist, inserts it before the closing `---`.
fn append_frontmatter_array(content: &str, field: &str, value: &str) -> String {
    let field_marker = format!("{field}:");
    if content.contains(&format!("\n{field_marker}")) || content.starts_with(&field_marker) {
        // Field exists — find it and append after the last list item or inline
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::with_capacity(lines.len() + 1);
        let mut in_frontmatter = false;
        let mut found_field = false;
        let mut inserted = false;

        for (i, line) in lines.iter().enumerate() {
            result.push(line.to_string());

            if line.trim() == "---" {
                in_frontmatter = !in_frontmatter;
                continue;
            }

            if in_frontmatter && line.starts_with(&field_marker) {
                found_field = true;
                // Check if next lines are list items
                let mut last_list_idx = i;
                for j in (i + 1)..lines.len() {
                    if lines[j].starts_with("  - ") {
                        last_list_idx = j;
                    } else {
                        break;
                    }
                }
                if last_list_idx == i {
                    // No existing list items, insert right after field line
                    result.push(format!("  - {value}"));
                    inserted = true;
                }
            }

            if found_field && !inserted && line.starts_with("  - ") {
                // Check if next line is NOT a list item — insert after this one
                let next = lines.get(i + 1);
                if next.is_none_or(|n| !n.starts_with("  - ")) {
                    result.push(format!("  - {value}"));
                    inserted = true;
                }
            }
        }

        let joined = result.join("\n");
        if content.ends_with('\n') {
            joined + "\n"
        } else {
            joined
        }
    } else {
        // Field doesn't exist — insert before closing ---
        let trimmed_start = if content.trim_start().starts_with("---") {
            content.find("---").unwrap_or(0) + 3
        } else {
            0
        };
        if let Some(close_pos) = content[trimmed_start..].find("\n---") {
            let insert_at = trimmed_start + close_pos;
            let new_field = format!("\n{field}:\n  - {value}");
            let mut result = content.to_string();
            result.insert_str(insert_at, &new_field);
            result
        } else {
            content.to_string()
        }
    }
}
