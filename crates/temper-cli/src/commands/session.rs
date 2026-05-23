use std::path::PathBuf;

use chrono::Local;
use serde::Serialize;
use temper_core::vault::Vault;

use crate::config::Config;
use crate::error::Result;
use crate::output;
use crate::vault;

/// Create or update today's session note.
///
/// Path: `<vault_root>/<context>/session/<date> — <slug>.md`
///
/// - If `context` is None and `task` is provided, infers context from the task
/// - If `context` is None and no task, falls back to "general"
/// - `title` defaults to today's date if omitted
/// - The filename uses a slugified version of the title
/// - If the session already exists and `stdin_content` is None: no-op (idempotent)
/// - If the session already exists and `stdin_content` is Some: replace body, preserve frontmatter
/// - If `task` is provided, task-linking is deferred to the cloud-mode follow-on (no local file)
/// - If `state` is also provided, updates the task's stage field
pub fn save(
    config: &Config,
    title: Option<&str>,
    context: Option<&str>,
    stdin_content: Option<&str>,
    task: Option<&str>,
    state: Option<&str>,
    format: &str,
) -> Result<()> {
    use temper_core::operations::{BodyUpdate, CreateResource, ResourceRef, UpdateResource};
    use temper_core::types::ManagedMeta;

    let today = Local::now().format("%Y-%m-%d").to_string();

    // Infer context from task if not explicitly provided
    let inferred_context = if context.is_none() {
        task.and_then(|slug| {
            crate::actions::task::find_task(config, slug, None)
                .ok()
                .flatten()
                .map(|info| info.context)
        })
    } else {
        None
    };
    let context_name = context
        .map(String::from)
        .or(inferred_context)
        .unwrap_or_else(|| "general".to_string());

    let note_title = title.unwrap_or(&today);
    let title_slug = vault::slugify(note_title);
    let slug = format!("{today}-{title_slug}");
    let owner = config.owner_for_context(&context_name);

    // Cloud-only exists-check: try resolve_by_uri; any error means "doesn't exist".
    let exists = session_exists(config, &context_name, &owner, &slug)?;

    let (runtime, backend, _client) = crate::backend_select::build_backend(config, &context_name)?;

    if exists {
        // Update path: only if stdin_content is provided, otherwise no-op.
        let Some(body) = stdin_content else {
            return Ok(());
        };
        let cmd = UpdateResource {
            resource: ResourceRef::scoped(owner.clone(), &context_name, "session", &slug),
            body: Some(BodyUpdate::new(body)),
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: temper_core::operations::Surface::CliCloud,
        };
        runtime.block_on(backend.update_resource(cmd))?;
        output::success(format!("Updated: {slug}"));
    } else {
        // Create path.
        let body_str = stdin_content.unwrap_or("");
        let cmd = CreateResource {
            slug: slug.clone(),
            doctype: "session".to_string(),
            context: context_name.clone(),
            title: note_title.to_string(),
            body: if body_str.is_empty() {
                None
            } else {
                Some(BodyUpdate::new(body_str))
            },
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: temper_core::operations::Surface::CliCloud,
        };
        runtime.block_on(backend.create_resource(cmd))?;

        if format == "json" {
            #[derive(Serialize)]
            struct SessionCreated<'a> {
                title: &'a str,
                context: &'a str,
                path: &'a str,
                date: &'a str,
            }
            let info = SessionCreated {
                title: note_title,
                context: &context_name,
                path: "",
                date: &today,
            };
            let json = serde_json::to_string_pretty(&info).unwrap_or_default();
            println!("{json}");
        } else {
            output::success(format!("Created: {slug}"));
        }

        // Task-linking requires the session file's frontmatter (temper-id), which
        // is not available in cloud mode. Deferred to cloud-mode follow-on task.
        if let Some(task_slug) = task {
            tracing::warn!(
                "skipping task-linking for cloud-mode session save (no local file to read); \
                 task={task_slug}, state={state:?}"
            );
        }
    }

    Ok(())
}

/// Check whether a session for the current day already exists.
///
/// Cloud-only: queries the API via `resolve_by_uri`. Any error (404 or
/// network) is treated as "doesn't exist".
fn session_exists(_config: &Config, context: &str, owner: &str, slug: &str) -> Result<bool> {
    let owner = owner.to_string();
    let context = context.to_string();
    let slug = slug.to_string();
    match crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .resolve_by_uri(&owner, &context, "session", &slug)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    }) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Show a single session's content.
///
/// Cloud-only: requires a context; resolves the session id via
/// `GET /api/resources/by-uri` and fetches content via
/// `GET /api/resources/{id}/content`. Also refreshes the local projection
/// file (best-effort). JSON emits a `SessionShow`-shaped struct.
pub fn show(
    config: &Config,
    slug_or_suffix: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    #[derive(Serialize)]
    struct SessionShow {
        date: String,
        context: String,
        title: String,
        path: String,
        content: String,
    }

    let ctx_s = context.map(str::to_string);
    let slug_s = slug_or_suffix.to_string();
    let config_clone = config.clone();

    let body = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = ctx_s.as_deref().ok_or_else(|| {
                crate::error::TemperError::Project(
                    "no context specified — use --context <name>".into(),
                )
            })?;
            let owner = config_clone.owner_for_context(ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, ctx, "session", &slug_s)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh — best-effort.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!(
                    "could not refresh projection file for '{slug_s}': {e}"
                ));
            }

            Ok(resp.markdown)
        })
    })?;

    if format == "json" {
        let ctx = context.unwrap_or("");
        let info = SessionShow {
            date: String::new(),
            context: ctx.to_string(),
            title: slug_or_suffix.to_string(),
            path: String::new(),
            content: body,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    print!("{body}");
    Ok(())
}

/// Return path that would be used for a session note (for testing/preview).
#[allow(dead_code)]
pub fn session_path(config: &Config, context: &str, title: &str) -> PathBuf {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let title_slug = vault::slugify(title);
    let slug = format!("{today}-{title_slug}");
    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    vault_layout.doc_file(&owner, context, "session", &slug)
}
