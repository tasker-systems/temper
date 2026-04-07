use askama::Template;
use chrono::Local;
use serde::Serialize;

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
    let _ = (config, slug, context, format);
    todo!("resource show is implemented in Task 5")
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
    let _ = (
        config, slug, doc_type, type_from, type_to, context, context_to, title, tags, aliases,
        relates_to, references, depends_on, stage, mode, effort, goal, seq, branch, pr, status,
    );
    todo!("resource update is implemented in Task 5")
}
