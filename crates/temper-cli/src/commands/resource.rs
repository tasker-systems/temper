use std::path::PathBuf;

use askama::Template;
use chrono::Local;
use serde::Serialize;
use temper_core::schema;
use temper_core::vault::Vault;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::{Result, TemperError};
use crate::format::OutputFormat;
use crate::output;
use crate::output::columns as col_registry;
use crate::output::table::TableRenderer;
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
///
/// In cloud mode (no local vault) we skip the vault-filesystem fallback
/// and trust the provided name directly — there are no context directories
/// on disk to check.
fn require_context(config: &Config, context: Option<&str>) -> Result<String> {
    use temper_core::types::config::VaultState;
    match context {
        Some(ctx) => {
            if matches!(VaultState::from_env(), VaultState::Cloud) {
                Ok(ctx.to_string())
            } else {
                Ok(super::resolve_context_with_fallback(config, ctx).into_owned())
            }
        }
        None => Err(TemperError::Project(
            "no context specified — use --context <name>".into(),
        )),
    }
}

/// Create a new resource.
#[expect(
    clippy::too_many_arguments,
    reason = "per-doctype creators have different required args; params struct deferred to Task 12+"
)]
pub fn create(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    goal: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    slug: Option<&str>,
    body_flag: Option<String>,
    format: &str,
) -> Result<()> {
    use std::io::IsTerminal;

    use temper_core::types::config::VaultState;

    validate_doc_type(doc_type)?;

    let ctx = require_context(config, context)?;

    let vault_state = VaultState::from_env();

    // Cloud-mode: skip vault writes; build IngestPayload and POST /api/ingest.
    #[cfg(feature = "embed")]
    if matches!(vault_state, VaultState::Cloud) {
        let stdin_is_tty = std::io::stdin().is_terminal();
        let body_opt = crate::actions::body_source::resolve_body_source(
            body_flag,
            stdin_is_tty,
            std::io::stdin(),
        )?;
        let body = body_opt.unwrap_or_else(|| format!("# {title}\n"));

        let managed_meta = crate::actions::frontmatter::build_managed_meta_for_create(
            crate::actions::frontmatter::NewResourceArgs {
                doc_type,
                context: &ctx,
                title,
                mode,
                effort,
                goal,
                stage: None,
                seq: None,
                status: None,
                provenance: None,
                llm_model: None,
                llm_run: None,
            },
        );

        let payload = crate::actions::ingest::build_ingest_payload(
            &body,
            title,
            &ctx,
            doc_type,
            None,
            Some(managed_meta),
            None,
        )?;

        let resource = crate::actions::runtime::with_client(|client| {
            Box::pin(async move {
                client
                    .ingest()
                    .create(&payload)
                    .await
                    .map_err(crate::actions::runtime::client_err_to_temper)
            })
        })?;

        if format == "json" {
            #[derive(serde::Serialize)]
            struct CloudCreated {
                id: String,
                slug: Option<String>,
                doc_type: String,
                context: String,
                title: String,
            }
            let info = CloudCreated {
                id: resource.id.to_string(),
                slug: resource.slug.clone(),
                doc_type: doc_type.to_string(),
                context: ctx.to_string(),
                title: resource.title.clone(),
            };
            let json = serde_json::to_string_pretty(&info).unwrap_or_default();
            println!("{json}");
        } else {
            let id_str = resource.id.to_string();
            let slug_display = resource.slug.as_deref().unwrap_or(&id_str);
            output::success(format!("Created: {slug_display}"));
        }
        return Ok(());
    }

    // Local-mode: existing vault-file create flow.
    // body_flag is intentionally unused in local mode (stdin piping handles body).
    let _ = body_flag;
    let _ = vault_state;

    let stdin_content = vault::read_stdin_if_piped();

    match doc_type {
        "task" => {
            let created_slug = crate::actions::task::create(
                config,
                &ctx,
                title,
                goal,
                mode,
                effort,
                stdin_content.as_deref(),
            )?;
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
        "research" => crate::commands::research::save(
            config,
            title,
            Some(ctx.as_str()),
            stdin_content.as_deref(),
            format,
        ),
        "concept" | "decision" => create_simple_resource(
            config,
            doc_type,
            title,
            &ctx,
            slug,
            stdin_content.as_deref(),
            format,
        ),
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
    stdin_content: Option<&str>,
    format: &str,
) -> Result<()> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let id = crate::ids::generate_id();
    let base_slug = vault::slugify(title);
    let slug = match slug_override {
        Some(s) => s.to_string(),
        // Concepts are identified by name (no date prefix); decisions get date prefix
        None => match doc_type {
            "concept" => base_slug,
            _ => format!("{today}-{base_slug}"),
        },
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

    // Parse the rendered template once, optionally replace body, then write.
    let mut fm = temper_core::frontmatter::Frontmatter::try_from(content.as_str())?;
    if let Some(body) = stdin_content {
        fm.set_body(body.to_string());
    }

    let vault_layout = Vault::new(&config.vault_root);
    let owner = config.owner_for_context(context);
    let dir = vault_layout.doc_type_dir(&owner, context, doc_type);
    let path = vault_layout.doc_file(&owner, context, doc_type, &slug);

    if path.exists() {
        return Err(TemperError::Vault(format!(
            "{doc_type} already exists: {slug}"
        )));
    }

    std::fs::create_dir_all(&dir).map_err(|e| TemperError::Vault(e.to_string()))?;
    fm.write_to(&path)?;

    crate::actions::runtime::publish_local_write_best_effort(&config.vault_root, &path)?;

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

// ---------------------------------------------------------------------------
// Unified resource list pipeline
// ---------------------------------------------------------------------------

/// A single resource row used by the unified list pipeline.
///
/// Carries the full frontmatter (for JSON output and column extraction) plus
/// the vault-relative path for debugging / future display needs.
#[derive(Debug, Clone)]
pub struct ResourceRow {
    /// Full parsed frontmatter as a JSON `Value`.
    pub frontmatter: serde_json::Value,
    /// Vault-relative path to the source markdown file.
    pub path: String,
}

impl ResourceRow {
    fn updated_at(&self) -> &str {
        self.frontmatter
            .get("temper-updated")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    #[cfg(test)]
    pub fn slug_for_tests(&self) -> String {
        self.frontmatter
            .get("slug")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
}

/// Filters applied after scanning and parsing rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListFilters<'a> {
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
}

/// Parameters for the public `list` command, bundled to keep the CLI entry
/// signature compact (and clippy happy).
#[derive(Debug, Clone, Copy)]
pub struct ListParams<'a> {
    pub doc_type: &'a str,
    pub context: Option<&'a str>,
    pub limit: Option<usize>,
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
    pub format: &'a str,
}

/// Parameters for `render_list`, the testable core of the unified list
/// pipeline. Bundling the inputs keeps the function under the project's
/// positional-argument limit and makes test call sites read naturally.
#[derive(Debug, Clone, Copy)]
pub struct RenderListParams<'a> {
    pub doc_type: &'a str,
    pub config: &'a Config,
    pub context: Option<&'a str>,
    pub limit: Option<usize>,
    pub filters: ListFilters<'a>,
    pub format: OutputFormat,
}

/// Scan disk for all resources of `doc_type`, optionally restricted to one
/// context. Returns one `ResourceRow` per `.md` file that has valid
/// frontmatter.
pub fn scan_rows(
    config: &Config,
    doc_type: &str,
    context: Option<&str>,
) -> Result<Vec<ResourceRow>> {
    let contexts_to_scan: Vec<String> = match context {
        Some(c) => vec![c.to_string()],
        None => config.contexts.clone(),
    };

    let mut rows = Vec::new();
    let vault_layout = Vault::new(&config.vault_root);
    for ctx in &contexts_to_scan {
        let owner = config.owner_for_context(ctx);
        let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
        if !dir.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| TemperError::Vault(e.to_string()))? {
            let entry = entry.map_err(|e| TemperError::Vault(e.to_string()))?;
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            if let Some(row) = parse_row(&path, &config.vault_root)? {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

/// Read one markdown file and convert its YAML frontmatter into a
/// `ResourceRow`. Files without valid frontmatter are skipped (`Ok(None)`).
fn parse_row(path: &std::path::Path, vault_root: &std::path::Path) -> Result<Option<ResourceRow>> {
    let content = std::fs::read_to_string(path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let Ok(fm) = temper_core::frontmatter::Frontmatter::try_from(content.as_str()) else {
        return Ok(None);
    };
    // Convert YAML value -> JSON value so downstream code can use
    // the existing `serde_json::Value`-based column registry.
    let frontmatter = serde_json::to_value(fm.value())
        .map_err(|e| TemperError::Vault(format!("frontmatter YAML→JSON: {e}")))?;
    let relative = path.strip_prefix(vault_root).unwrap_or(path);
    Ok(Some(ResourceRow {
        frontmatter,
        path: relative.to_string_lossy().to_string(),
    }))
}

/// Sort rows by `temper-updated` descending (most recent first). Rows missing
/// the field sort to the end.
pub fn sort_rows(rows: &mut [ResourceRow]) {
    rows.sort_by(|a, b| b.updated_at().cmp(a.updated_at()));
}

/// Apply stage/goal/status filters, dropping rows that don't match.
pub fn filter_rows(rows: Vec<ResourceRow>, filters: ListFilters<'_>) -> Vec<ResourceRow> {
    rows.into_iter()
        .filter(|row| match_filters(row, &filters))
        .collect()
}

fn match_filters(row: &ResourceRow, filters: &ListFilters<'_>) -> bool {
    if let Some(stage) = filters.stage {
        if row.frontmatter.get("temper-stage").and_then(|v| v.as_str()) != Some(stage) {
            return false;
        }
    }
    if let Some(goal) = filters.goal {
        if row.frontmatter.get("temper-goal").and_then(|v| v.as_str()) != Some(goal) {
            return false;
        }
    }
    if let Some(status) = filters.status {
        if row
            .frontmatter
            .get("temper-status")
            .and_then(|v| v.as_str())
            != Some(status)
        {
            return false;
        }
    }
    true
}

/// Cloud-first list: call the server, return rows sorted server-side
/// (`ORDER BY updated DESC`).
async fn fetch_list_rows(
    client: &temper_client::TemperClient,
    doc_type: &str,
    context: Option<&str>,
    limit: usize,
) -> Result<Vec<temper_core::types::resource::ResourceRow>> {
    use temper_core::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let params = ResourceListParams {
        doc_type_name: Some(doc_type.to_string()),
        context_name: context.map(ToString::to_string),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        ..Default::default()
    };
    let resp = client
        .resources()
        .list(&params)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    Ok(resp.rows)
}

/// Map a server `ResourceRow` to the frontmatter-shaped `serde_json::Value`
/// that `col_registry::extract_row` expects. The registry was built for
/// local scan_rows output; we adapt the server row shape to the same
/// keys so rendering is unchanged.
fn row_to_frontmatter_value(row: &temper_core::types::resource::ResourceRow) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("title".into(), serde_json::Value::String(row.title.clone()));
    if let Some(slug) = &row.slug {
        map.insert("slug".into(), serde_json::Value::String(slug.clone()));
    }
    map.insert(
        "temper-updated".into(),
        serde_json::Value::String(row.updated.to_rfc3339()),
    );
    map.insert(
        "temper-context".into(),
        serde_json::Value::String(row.context_name.clone()),
    );
    map.insert(
        "temper-type".into(),
        serde_json::Value::String(row.doc_type_name.clone()),
    );
    if let Some(stage) = &row.stage {
        map.insert(
            "temper-stage".into(),
            serde_json::Value::String(stage.clone()),
        );
    }
    if let Some(mode) = &row.mode {
        map.insert(
            "temper-mode".into(),
            serde_json::Value::String(mode.clone()),
        );
    }
    if let Some(effort) = &row.effort {
        map.insert(
            "temper-effort".into(),
            serde_json::Value::String(effort.clone()),
        );
    }
    if let Some(seq) = row.seq {
        map.insert("temper-seq".into(), serde_json::Value::Number(seq.into()));
    }
    serde_json::Value::Object(map)
}

/// Render server rows using the same per-doctype column registry used
/// by the local-mode `render_list`. This keeps table output shape
/// stable between the two modes.
fn render_server_rows(
    doc_type: &str,
    rows: &[temper_core::types::resource::ResourceRow],
    format: OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(
            &rows
                .iter()
                .map(row_to_frontmatter_value)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default()),
        OutputFormat::Pretty | OutputFormat::NoTty => {
            let columns = col_registry::display_columns(doc_type);
            if columns.is_empty() || rows.is_empty() {
                return Ok(String::new());
            }
            let mut renderer = TableRenderer::new(columns.clone());
            for row in rows {
                let fm_value = row_to_frontmatter_value(row);
                renderer.push_row(col_registry::extract_row(&fm_value, &columns));
            }
            Ok(if format == OutputFormat::Pretty {
                renderer.render_pretty()
            } else {
                renderer.render_no_tty()
            })
        }
    }
}

/// Render the unified list pipeline to a `String` for the given format.
///
/// This is the testable core used by both `list()` (CLI entry) and the
/// integration tests below.
///
/// Empty-result handling: JSON always emits `[]` (machine-friendly),
/// Pretty/NoTty return an empty string so the caller (`list()`) can surface
/// a user-friendly "No X resources found" hint instead of a bare header row.
pub fn render_list(params: &RenderListParams<'_>) -> Result<String> {
    validate_doc_type(params.doc_type)?;
    // Filter first, then sort — sorting unfiltered rows wastes work on rows
    // we're about to discard.
    let rows = scan_rows(params.config, params.doc_type, params.context)?;
    let mut rows = filter_rows(rows, params.filters);
    sort_rows(&mut rows);
    rows.truncate(params.limit.unwrap_or(20));

    match params.format {
        OutputFormat::Json => {
            let frontmatters: Vec<&serde_json::Value> =
                rows.iter().map(|r| &r.frontmatter).collect();
            Ok(serde_json::to_string_pretty(&frontmatters).unwrap_or_default())
        }
        OutputFormat::Pretty | OutputFormat::NoTty => {
            // Empty result → return empty string so the CLI entry can render
            // a "No X resources found" hint instead of a header-only table.
            if rows.is_empty() {
                return Ok(String::new());
            }
            let columns = col_registry::display_columns(params.doc_type);
            if columns.is_empty() {
                return Ok(String::new());
            }
            let mut renderer = TableRenderer::new(columns.clone());
            for row in &rows {
                renderer.push_row(col_registry::extract_row(&row.frontmatter, &columns));
            }
            if params.format == OutputFormat::Pretty {
                Ok(renderer.render_pretty())
            } else {
                Ok(renderer.render_no_tty())
            }
        }
    }
}

/// List resources of a given type (unified pipeline for all doc types).
pub fn list(config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::config::VaultState;

    // Hints for filters that only apply to certain types (unchanged).
    if params.stage.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--stage filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.goal.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--goal filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.status.is_some() && params.doc_type != "goal" {
        output::hint(format!(
            "--status filter is only meaningful for goals; ignored for {}.",
            params.doc_type
        ));
    }

    if let Some(s) = params.stage {
        if params.doc_type == "task" {
            vault::validate_stage(s)?;
        }
    }

    let format = OutputFormat::parse(params.format);
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = params.limit.unwrap_or(20);

    let vault_state = VaultState::from_env();

    // Attempt server-first. Fall back to local scan on network error
    // in Local mode only; Cloud mode surfaces the error.
    let rows_result = runtime::with_client(move |client| {
        Box::pin(async move { fetch_list_rows(client, &doc_type, context.as_deref(), limit).await })
    });

    let server_rows = match (rows_result, vault_state) {
        (Ok(rows), _) => Some(rows),
        // Local mode: fall back to the local vault scan only when the server
        // is unreachable. Server-originated errors (4xx/5xx, auth) surface
        // as-is — silently masking them with stale local data would hide
        // real problems.
        (Err(e @ TemperError::Network(_)), VaultState::Local) => {
            output::hint(format!(
                "cloud unreachable: {e}. Falling back to local scan."
            ));
            None
        }
        (Err(e), _) => return Err(e),
    };

    let body = match server_rows {
        Some(rows) => render_server_rows(params.doc_type, &rows, format)?,
        None => render_list(&RenderListParams {
            doc_type: params.doc_type,
            config,
            context: params.context,
            limit: params.limit,
            filters: ListFilters {
                stage: if params.doc_type == "task" {
                    params.stage
                } else {
                    None
                },
                goal: if params.doc_type == "task" {
                    params.goal
                } else {
                    None
                },
                status: if params.doc_type == "goal" {
                    params.status
                } else {
                    None
                },
            },
            format,
        })?,
    };

    if body.trim().is_empty() {
        output::hint(format!("No {} resources found.", params.doc_type));
        return Ok(());
    }

    output::plain(body.trim_end());
    Ok(())
}

/// Show a resource's content.
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    edges: bool,
) -> Result<()> {
    validate_doc_type(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }?;

    if edges {
        show_edges(slug, format)?;
    }

    Ok(())
}

/// Resolve `(doc_type, slug, context)` to a `ResourceId`.
///
/// In Local mode, fast-paths through the local file's `temper-id` frontmatter
/// field when the file exists. Falls back to `GET /api/resources/by-uri` (the
/// slow path) in Cloud mode or when the local file has no canonical id yet.
///
/// `pub(crate)` so that `task::show` and `session::show` share this logic
/// without duplication.
pub(crate) async fn resolve_resource_id(
    config: &Config,
    client: &temper_client::TemperClient,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    vault_state: temper_core::types::VaultState,
) -> Result<temper_core::types::ids::ResourceId> {
    use temper_core::types::ids::ResourceId;

    if matches!(vault_state, temper_core::types::VaultState::Local) {
        if let Ok((path, _)) = find_resource_file(config, doc_type, slug, context) {
            let body =
                std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
            if let Ok(fm) = temper_core::frontmatter::Frontmatter::try_from(body.as_str()) {
                if let Some(id_str) = fm.value().get("temper-id").and_then(|v| v.as_str()) {
                    if let Ok(uuid) = uuid::Uuid::parse_str(id_str) {
                        return Ok(ResourceId::from(uuid));
                    }
                }
            }
        }
    }

    let ctx = require_context(config, context)?;
    let owner = config.owner_for_context(&ctx);
    let row = client
        .resources()
        .resolve_by_uri(&owner, &ctx, doc_type, slug)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    Ok(row.id)
}

/// Return the existing local path for a resource if found, or compute where
/// it would live based on `Vault::doc_file`.
fn find_or_compute_local_path(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
) -> Result<(std::path::PathBuf, String)> {
    if let Ok((path, ctx)) = find_resource_file(config, doc_type, slug, context) {
        return Ok((path, ctx));
    }
    let ctx = require_context(config, context)?;
    let owner = config.owner_for_context(&ctx);
    let vault_layout = Vault::new(&config.vault_root);
    let path = vault_layout.doc_file(&owner, &ctx, doc_type, slug);
    Ok((path, ctx))
}

/// Render generic resource output in the requested format.
///
/// `local_path` is `None` in Cloud mode (no file on disk).
fn render_generic_output(
    doc_type: &str,
    slug: &str,
    context: &str,
    config: &Config,
    local_path: Option<&std::path::Path>,
    body: String,
    format: &str,
) -> Result<()> {
    if format == "json" {
        let fm = temper_core::frontmatter::Frontmatter::try_from(body.as_str()).ok();
        let title = fm
            .as_ref()
            .and_then(|f| f.value().get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or(slug);
        let path_str = local_path
            .and_then(|p| p.strip_prefix(&config.vault_root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();

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
            context,
            path: path_str,
            content: body,
        };
        let json = serde_json::to_string_pretty(&info).unwrap_or_default();
        println!("{json}");
        return Ok(());
    }

    print!("{body}");
    Ok(())
}

/// Show a generic resource (goal, research, concept, decision).
///
/// In Local mode: resolves an id from frontmatter or by-uri, then uses the
/// three-tier freshness ladder (`show_cache::fetch`) before rendering.
/// In Cloud mode: fetches content directly from the API with no disk write.
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::{runtime, show_cache};
    use std::time::Duration;
    use temper_core::types::VaultState;

    let vault_state = VaultState::from_env();
    let doc_type_s = doc_type.to_string();
    let slug_s = slug.to_string();
    let context_owned = context.map(str::to_string);
    let format_s = format.to_string();

    match vault_state {
        VaultState::Cloud => {
            let config_clone = config.clone();
            let doc_type_inner = doc_type_s.clone();
            let slug_inner = slug_s.clone();
            let ctx_inner = context_owned.clone();

            let body = runtime::with_client(|client| {
                Box::pin(async move {
                    let id = resolve_resource_id(
                        &config_clone,
                        client,
                        &doc_type_inner,
                        &slug_inner,
                        ctx_inner.as_deref(),
                        VaultState::Cloud,
                    )
                    .await?;
                    let resp = client
                        .resources()
                        .content(*id.as_uuid())
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)?;
                    Ok(resp.markdown)
                })
            })?;

            let ctx = context_owned.unwrap_or_default();
            render_generic_output(&doc_type_s, &slug_s, &ctx, config, None, body, &format_s)
        }
        VaultState::Local => {
            let config_clone = config.clone();
            let doc_type_inner = doc_type_s.clone();
            let slug_inner = slug_s.clone();
            let ctx_inner = context_owned.clone();

            let (path, ctx) = find_or_compute_local_path(config, &doc_type_s, &slug_s, context)?;

            // Tier 0: debounce check before spinning up the runtime.
            // If the file is fresh we serve it immediately; no network, no
            // resource id resolution needed.
            if let Some(body) = show_cache::read_if_fresh(
                &path,
                std::time::Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
            )? {
                return render_generic_output(
                    &doc_type_s,
                    &slug_s,
                    &ctx,
                    config,
                    Some(&path),
                    body,
                    &format_s,
                );
            }

            let (body, local_path_for_render) = runtime::with_client(|client| {
                Box::pin(async move {
                    let id = resolve_resource_id(
                        &config_clone,
                        client,
                        &doc_type_inner,
                        &slug_inner,
                        ctx_inner.as_deref(),
                        VaultState::Local,
                    )
                    .await?;
                    let result = show_cache::fetch(show_cache::ShowCacheParams {
                        client,
                        resource_id: id,
                        local_path: &path,
                        debounce: Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
                    })
                    .await?;
                    Ok((result.content, path))
                })
            })?;

            render_generic_output(
                &doc_type_s,
                &slug_s,
                &ctx,
                config,
                Some(&local_path_for_render),
                body,
                &format_s,
            )
        }
    }
}

/// Fetch and display edges for a resource via the API.
fn show_edges(slug: &str, format: &str) -> Result<()> {
    use crate::actions::runtime;

    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let resource_id = manifest
        .entries
        .iter()
        .find(|(_, entry)| {
            entry
                .path
                .strip_suffix(".md")
                .and_then(|p| p.rsplit('/').next())
                == Some(slug)
        })
        .map(|(id, _)| uuid::Uuid::from(*id))
        .ok_or_else(|| {
            TemperError::Vault(format!(
                "resource '{slug}' not found in manifest — sync first to use --edges"
            ))
        })?;

    let edges: Vec<temper_core::types::graph::GraphEdgeRow> = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .edges(resource_id)
                .await
                .map_err(crate::commands::client_err)
        })
    })?;

    if edges.is_empty() {
        if format != "json" {
            println!("\nEdges: (none)");
        }
        return Ok(());
    }

    if format == "json" {
        let json = serde_json::to_string_pretty(&edges).unwrap_or_default();
        println!("{json}");
    } else {
        println!("\nEdges:");
        let outgoing: Vec<_> = edges.iter().filter(|e| e.direction == "outgoing").collect();
        let incoming: Vec<_> = edges.iter().filter(|e| e.direction == "incoming").collect();

        if !outgoing.is_empty() {
            println!("  outgoing:");
            for e in &outgoing {
                println!(
                    "    {} \u{2192} {} ({})",
                    e.edge_type, e.peer_slug, e.peer_title
                );
            }
        }
        if !incoming.is_empty() {
            println!("  incoming:");
            for e in &incoming {
                println!(
                    "    {} \u{2190} {} ({})",
                    e.edge_type, e.peer_slug, e.peer_title
                );
            }
        }
    }

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
    let vault_layout = Vault::new(&config.vault_root);

    for ctx in &contexts_to_scan {
        let owner = config.owner_for_context(ctx);
        let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
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

/// Parameters for resource update.
pub struct UpdateParams<'a> {
    pub slug: &'a str,
    pub doc_type: Option<&'a str>,
    pub type_from: Option<&'a str>,
    pub type_to: Option<&'a str>,
    pub context: Option<&'a str>,
    pub context_to: Option<&'a str>,
    // Base schema fields
    pub title: Option<&'a str>,
    pub tags: &'a [String],
    pub aliases: &'a [String],
    pub relates_to: &'a [String],
    pub references: &'a [String],
    pub depends_on: &'a [String],
    pub extends: &'a [String],
    pub preceded_by: &'a [String],
    pub derived_from: &'a [String],
    // Task-specific fields
    pub stage: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub seq: Option<i64>,
    pub branch: Option<&'a str>,
    pub pr: Option<&'a str>,
    // Goal-specific fields
    pub status: Option<&'a str>,
    /// Body source: None (auto-detect stdin), Some("-") (explicit stdin), or Some("@\<path\>")
    pub body: Option<String>,
}

/// Build a partial `ManagedMeta` from update CLI flags. Returns `None` if no
/// managed-meta-mutating flags were passed.
#[cfg(feature = "embed")]
fn build_partial_managed_meta_from_args(
    params: &UpdateParams<'_>,
) -> Option<temper_core::types::ManagedMeta> {
    let any_set = params.stage.is_some()
        || params.mode.is_some()
        || params.effort.is_some()
        || params.goal.is_some()
        || params.seq.is_some()
        || params.branch.is_some()
        || params.pr.is_some()
        || params.status.is_some();
    if !any_set {
        return None;
    }
    Some(temper_core::types::ManagedMeta {
        stage: params.stage.map(String::from),
        mode: params.mode.map(String::from),
        effort: params.effort.map(String::from),
        goal: params.goal.map(String::from),
        seq: params.seq,
        branch: params.branch.map(String::from),
        pr: params.pr.map(String::from),
        status: params.status.map(String::from),
        ..Default::default()
    })
}

/// Build a partial `open_meta` JSON object from update CLI list flags. Returns
/// `None` if no open-meta list flags were passed.
#[cfg(feature = "embed")]
fn build_partial_open_meta_from_args(params: &UpdateParams<'_>) -> Option<serde_json::Value> {
    let mut obj = serde_json::Map::new();
    if !params.tags.is_empty() {
        obj.insert("tags".to_string(), serde_json::json!(params.tags));
    }
    if !params.aliases.is_empty() {
        obj.insert("aliases".to_string(), serde_json::json!(params.aliases));
    }
    if !params.relates_to.is_empty() {
        obj.insert(
            "relates-to".to_string(),
            serde_json::json!(params.relates_to),
        );
    }
    if !params.references.is_empty() {
        obj.insert(
            "references".to_string(),
            serde_json::json!(params.references),
        );
    }
    if !params.depends_on.is_empty() {
        obj.insert(
            "depends-on".to_string(),
            serde_json::json!(params.depends_on),
        );
    }
    if !params.extends.is_empty() {
        obj.insert("extends".to_string(), serde_json::json!(params.extends));
    }
    if !params.preceded_by.is_empty() {
        obj.insert(
            "preceded-by".to_string(),
            serde_json::json!(params.preceded_by),
        );
    }
    if !params.derived_from.is_empty() {
        obj.insert(
            "derived-from".to_string(),
            serde_json::json!(params.derived_from),
        );
    }
    if obj.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(obj))
    }
}

/// Cloud-mode `temper resource update` — no vault file is touched. Resolves
/// the resource id via the API, builds a partial `ResourceUpdateRequest`
/// (managed_meta + open_meta + optional body trio), and posts
/// `PATCH /api/resources/{id}`. Prints `{slug, content_hash}` for the
/// agent's next show-edit-cat cycle.
#[cfg(feature = "embed")]
fn cloud_mode_update(config: &Config, params: &UpdateParams<'_>, current_type: &str) -> Result<()> {
    use std::io::IsTerminal;

    // Resolve body source first (sync, doesn't need the runtime).
    let stdin_is_tty = std::io::stdin().is_terminal();
    let body_opt = crate::actions::body_source::resolve_body_source(
        params.body.clone(),
        stdin_is_tty,
        std::io::stdin(),
    )?;

    let (content, content_hash, chunks_packed) = match body_opt {
        Some(b) => {
            let chunks = crate::actions::ingest::compute_body_chunks(&b)?;
            (
                Some(b),
                Some(chunks.content_hash),
                Some(chunks.chunks_packed),
            )
        }
        None => (None, None, None),
    };

    let managed_meta = build_partial_managed_meta_from_args(params);
    let open_meta = build_partial_open_meta_from_args(params);

    let req = temper_core::types::ResourceUpdateRequest {
        title: params.title.map(String::from),
        slug: None,
        managed_meta,
        open_meta,
        content,
        content_hash,
        chunks_packed,
    };

    // Compute owned values before the async block so the closure is 'static.
    // In cloud mode, slug→id resolution goes through the API's by-uri lookup;
    // we only need owner + context strings to form the URI.
    let ctx = require_context(config, params.context)?.to_string();
    let owner = config.owner_for_context(&ctx).to_string();
    let doc_type = current_type.to_string();
    let slug = params.slug.to_string();

    let updated = crate::actions::runtime::with_client(move |client| {
        let req = req.clone();
        let owner = owner.clone();
        let ctx = ctx.clone();
        let doc_type = doc_type.clone();
        let slug = slug.clone();
        Box::pin(async move {
            let row = client
                .resources()
                .resolve_by_uri(&owner, &ctx, &doc_type, &slug)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            client
                .resources()
                .update(*row.id, &req)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let slug_display = updated
        .slug
        .as_deref()
        .unwrap_or(&updated.id.to_string())
        .to_string();
    let hash_display = updated.body_hash.as_deref().unwrap_or("").to_string();
    println!(
        "{}",
        serde_json::json!({
            "slug": slug_display,
            "content_hash": hash_display,
        })
    );
    Ok(())
}

/// Update a resource's frontmatter fields.
pub fn update(config: &Config, params: &UpdateParams<'_>) -> Result<()> {
    use temper_core::types::config::VaultState;

    // Resolve current type from --type or --type-from (one is required)
    let current_type = params
        .doc_type
        .or(params.type_from)
        .ok_or_else(|| TemperError::Project("--type or --type-from is required".into()))?;
    validate_doc_type(current_type)?;

    if let Some(tt) = params.type_to {
        validate_doc_type(tt)?;
    }

    let vault_state = VaultState::from_env();

    // Cloud-mode: skip vault file edits; build ResourceUpdateRequest and PATCH /api/resources/{id}.
    #[cfg(feature = "embed")]
    if matches!(vault_state, VaultState::Cloud) {
        return cloud_mode_update(config, params, current_type);
    }
    #[cfg(not(feature = "embed"))]
    let _ = vault_state;

    // Find the resource file
    let (path, ctx) = find_resource_file(config, current_type, params.slug, params.context)?;

    // Load updatable fields from schema for validation
    let schema_fields = schema::updatable_fields(current_type)?;

    // Build list of scalar field updates: (frontmatter_key, value)
    let scalar_updates: Vec<(&str, String)> = [
        ("title", params.title.map(String::from)),
        ("temper-stage", params.stage.map(String::from)),
        ("temper-mode", params.mode.map(String::from)),
        ("temper-effort", params.effort.map(String::from)),
        ("temper-goal", params.goal.map(String::from)),
        ("temper-branch", params.branch.map(String::from)),
        ("temper-pr", params.pr.map(String::from)),
        ("temper-status", params.status.map(String::from)),
        ("temper-seq", params.seq.map(|s| s.to_string())),
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

    // Parse the file once, apply all mutations to the aggregate, then write
    // exactly once to the (potentially moved) final path.
    let mut fm = temper_core::frontmatter::Frontmatter::parse_file(&path)?;

    // Apply scalar field updates
    for (field_name, value) in &scalar_updates {
        fm.set_managed_field(field_name, serde_json::Value::String(value.clone()));
    }

    // Apply array field appends. Uses canonical (underscore) open-field
    // names — Frontmatter::try_from has already normalized hyphen aliases
    // at the parse boundary, so looking up by canonical form finds the
    // existing sequence.
    let array_updates: Vec<(&str, &[String])> = vec![
        ("tags", params.tags),
        ("aliases", params.aliases),
        ("relates_to", params.relates_to),
        ("references", params.references),
        ("depends_on", params.depends_on),
        ("extends", params.extends),
        ("preceded_by", params.preceded_by),
        ("derived_from", params.derived_from),
    ];
    for (field_name, values) in &array_updates {
        for value in *values {
            let mapping = fm
                .value_mut()
                .as_mapping_mut()
                .expect("Frontmatter invariant: value is a mapping");
            let key = serde_yaml::Value::String((*field_name).to_string());
            let new_entry = serde_yaml::Value::String(value.clone());
            match mapping.get_mut(&key) {
                Some(serde_yaml::Value::Sequence(seq)) => seq.push(new_entry),
                _ => {
                    mapping.insert(key, serde_yaml::Value::Sequence(vec![new_entry]));
                }
            }
        }
    }

    // Handle --context-to (move file to new context dir, update temper-context)
    let vault_layout = Vault::new(&config.vault_root);
    let final_ctx;
    let mut final_path = path.clone();
    if let Some(new_ctx) = params.context_to {
        let new_owner = config.owner_for_context(new_ctx);
        let new_dir =
            vault_layout.doc_type_dir(&new_owner, new_ctx, params.type_to.unwrap_or(current_type));
        std::fs::create_dir_all(&new_dir)?;
        let filename = path
            .file_name()
            .ok_or_else(|| TemperError::Vault("cannot determine filename".into()))?;
        let new_path = new_dir.join(filename);
        fm.set_managed_field(
            "temper-context",
            serde_json::Value::String(new_ctx.to_string()),
        );
        final_path = new_path;
        final_ctx = new_ctx.to_string();
    } else {
        final_ctx = ctx.clone();
    }

    // Handle --type-to (move file to new type dir, update temper-type)
    if let Some(new_type) = params.type_to {
        let target_ctx = params.context_to.unwrap_or(&final_ctx);
        let target_owner = config.owner_for_context(target_ctx);
        let new_dir = vault_layout.doc_type_dir(&target_owner, target_ctx, new_type);
        std::fs::create_dir_all(&new_dir)?;
        let filename = final_path
            .file_name()
            .ok_or_else(|| TemperError::Vault("cannot determine filename".into()))?;
        let new_path = new_dir.join(filename);
        fm.set_managed_field(
            "temper-type",
            serde_json::Value::String(new_type.to_string()),
        );
        final_path = new_path;
    }

    // Update temper-updated timestamp
    let datetime = Local::now().to_rfc3339();
    fm.set_managed_field(
        "temper-updated",
        serde_json::Value::String(datetime.clone()),
    );

    // Write the mutated frontmatter to the (possibly moved) final path.
    fm.write_to(&final_path)?;

    // If file was moved, remove old file
    if final_path != path && path.exists() {
        std::fs::remove_file(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
    }

    crate::actions::runtime::publish_local_write_best_effort(&config.vault_root, &final_path)?;

    // Determine slug for output
    let final_slug = final_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Emit ResourceUpdate discovery event
    let event = Event::ResourceUpdate {
        ts: datetime,
        doc_type: params.type_to.unwrap_or(current_type).to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // validate_doc_type tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_doc_type_valid_types() {
        for doc_type in &["task", "goal", "session", "research", "concept", "decision"] {
            assert!(
                validate_doc_type(doc_type).is_ok(),
                "expected '{doc_type}' to be valid"
            );
        }
    }

    #[test]
    fn validate_doc_type_invalid_returns_error() {
        let result = validate_doc_type("foo");
        assert!(result.is_err(), "expected 'foo' to be invalid");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("invalid resource type"),
            "error should mention invalid resource type: {err_msg}"
        );
        assert!(
            err_msg.contains("foo"),
            "error should include the invalid value: {err_msg}"
        );
    }

    #[test]
    fn validate_doc_type_empty_string_returns_error() {
        assert!(validate_doc_type("").is_err());
    }
}

#[cfg(test)]
mod list_pipeline_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_config(tmp: &TempDir) -> Config {
        let vault_root = tmp.path().to_path_buf();
        fs::create_dir_all(vault_root.join(".temper")).unwrap();
        Config {
            state_dir: vault_root.join(".temper"),
            vault_root,
            contexts: vec!["temper".into(), "default".into()],
            subscriptions: Vec::new(),
            skill_output: PathBuf::from("/tmp/skill"),
        }
    }

    fn write_resource(
        config: &Config,
        ctx: &str,
        doc_type: &str,
        slug: &str,
        updated: &str,
        extras: &str,
    ) {
        let vault_layout = Vault::new(&config.vault_root);
        let owner = config.owner_for_context(ctx);
        let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
        fs::create_dir_all(&dir).unwrap();
        let content = format!(
            "---\ntemper-id: \"id-{slug}\"\ntemper-type: {doc_type}\ntemper-context: {ctx}\nslug: {slug}\ntitle: \"Title {slug}\"\ntemper-updated: \"{updated}\"\n{extras}---\n\nbody\n"
        );
        fs::write(dir.join(format!("{slug}.md")), content).unwrap();
    }

    #[test]
    fn scan_rows_sorts_descending_by_updated() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "a",
            "2026-04-01T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );
        write_resource(
            &config,
            "temper",
            "task",
            "b",
            "2026-04-07T00:00:00Z",
            "temper-stage: done\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let mut sorted = rows;
        sort_rows(&mut sorted);
        assert_eq!(sorted[0].slug_for_tests(), "b");
        assert_eq!(sorted[1].slug_for_tests(), "a");
    }

    #[test]
    fn scan_rows_skips_non_markdown_and_files_without_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        let vault_layout = Vault::new(&config.vault_root);
        let owner = config.owner_for_context("temper");
        let dir = vault_layout.doc_type_dir(&owner, "temper", "task");
        fs::create_dir_all(&dir).unwrap();
        // Non-md file
        fs::write(dir.join("notes.txt"), "not markdown").unwrap();
        // Markdown without frontmatter
        fs::write(dir.join("raw.md"), "no frontmatter here").unwrap();
        // Valid resource
        write_resource(
            &config,
            "temper",
            "task",
            "ok",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slug_for_tests(), "ok");
    }

    #[test]
    fn scan_rows_all_contexts_when_none_specified() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "session",
            "one",
            "2026-04-01T00:00:00Z",
            "",
        );
        write_resource(
            &config,
            "default",
            "session",
            "two",
            "2026-04-02T00:00:00Z",
            "",
        );

        let rows = scan_rows(&config, "session", None).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn filter_rows_respects_stage() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "x",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );
        write_resource(
            &config,
            "temper",
            "task",
            "y",
            "2026-04-07T00:00:00Z",
            "temper-stage: in-progress\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let filtered = filter_rows(
            rows,
            ListFilters {
                stage: Some("in-progress"),
                goal: None,
                status: None,
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "y");
    }

    #[test]
    fn filter_rows_respects_goal() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "in-core",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );
        write_resource(
            &config,
            "temper",
            "task",
            "in-other",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: other\ntemper-mode: build\ntemper-effort: small\n",
        );

        let rows = scan_rows(&config, "task", Some("temper")).unwrap();
        let filtered = filter_rows(
            rows,
            ListFilters {
                stage: None,
                goal: Some("core"),
                status: None,
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "in-core");
    }

    #[test]
    fn filter_rows_respects_status() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "goal",
            "g1",
            "2026-04-07T00:00:00Z",
            "temper-status: active\ntemper-seq: 10\n",
        );
        write_resource(
            &config,
            "temper",
            "goal",
            "g2",
            "2026-04-07T00:00:00Z",
            "temper-status: completed\ntemper-seq: 20\n",
        );

        let rows = scan_rows(&config, "goal", Some("temper")).unwrap();
        let filtered = filter_rows(
            rows,
            ListFilters {
                stage: None,
                goal: None,
                status: Some("completed"),
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].slug_for_tests(), "g2");
    }

    #[test]
    fn truncate_to_limit() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        for i in 0..5 {
            write_resource(
                &config,
                "temper",
                "session",
                &format!("s{i}"),
                &format!("2026-04-0{}T00:00:00Z", i + 1),
                "",
            );
        }
        let mut rows = scan_rows(&config, "session", Some("temper")).unwrap();
        sort_rows(&mut rows);
        rows.truncate(2);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn render_no_tty_emits_tab_header_and_rows() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "only",
            "2026-04-07T00:00:00Z",
            "temper-stage: in-progress\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::NoTty,
        })
        .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            "Context\tType\tSlug\tUpdated\tStage\tMode\tEffort\tGoal"
        );
        assert!(lines[1].starts_with("temper\ttask\tonly\t2026-04-07"));
    }

    #[test]
    fn render_pretty_has_table_structure() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "task",
            "alpha",
            "2026-04-07T00:00:00Z",
            "temper-stage: backlog\ntemper-goal: core\ntemper-mode: build\ntemper-effort: small\n",
        );

        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Pretty,
        })
        .unwrap();
        let lines: Vec<&str> = out.lines().collect();
        // header | separator | 1 data row
        assert_eq!(lines.len(), 3, "expected 3 lines got: {out}");
        assert!(lines[0].starts_with('|'), "header should start with pipe");
        assert!(lines[1].contains("---"), "separator row contains dashes");
        assert!(lines[2].contains("alpha"), "data row should contain slug");
    }

    #[test]
    fn render_json_emits_full_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        write_resource(
            &config,
            "temper",
            "research",
            "note",
            "2026-04-07T00:00:00Z",
            "",
        );
        let out = render_list(&RenderListParams {
            doc_type: "research",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Json,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["slug"], "note");
        assert_eq!(arr[0]["title"], "Title note");
        assert_eq!(arr[0]["temper-type"], "research");
    }

    #[test]
    fn render_json_empty_list_emits_empty_array() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Json,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, serde_json::json!([]));
    }

    #[test]
    fn render_list_empty_pretty_returns_empty_string() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        // No resources written — the pipeline should return an empty body
        // so the CLI entry can render a "No X resources found" hint instead
        // of a header-only table.
        let out = render_list(&RenderListParams {
            doc_type: "task",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::Pretty,
        })
        .unwrap();
        assert!(
            out.trim().is_empty(),
            "expected empty body for Pretty empty-list, got: {out:?}"
        );
    }

    #[test]
    fn render_list_empty_no_tty_returns_empty_string() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        let out = render_list(&RenderListParams {
            doc_type: "goal",
            config: &config,
            context: Some("temper"),
            limit: None,
            filters: ListFilters::default(),
            format: OutputFormat::NoTty,
        })
        .unwrap();
        assert!(
            out.trim().is_empty(),
            "expected empty body for NoTty empty-list, got: {out:?}"
        );
    }

    #[test]
    fn render_list_respects_limit() {
        let tmp = TempDir::new().unwrap();
        let config = make_config(&tmp);
        for i in 0..5 {
            write_resource(
                &config,
                "temper",
                "session",
                &format!("s{i}"),
                &format!("2026-04-0{}T00:00:00Z", i + 1),
                "",
            );
        }
        let out = render_list(&RenderListParams {
            doc_type: "session",
            config: &config,
            context: Some("temper"),
            limit: Some(2),
            filters: ListFilters::default(),
            format: OutputFormat::Json,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
    }
}
