use chrono::Local;
use temper_core::schema;

use crate::config::Config;
use crate::discovery::{self, Event};
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;

/// Flat result emitted by `temper resource create`.
///
/// `ResourceRow` is flattened so all wire-type fields appear at the top level
/// alongside `status`. Breaking change (Task 9): replaces the 7-variant
/// per-doctype JSON shape map (Task/Goal/Session/Research/Concept/Decision/default).
#[derive(Debug, serde::Serialize)]
pub(crate) struct CreateActionResult {
    pub status: &'static str,
    #[serde(flatten)]
    pub resource: temper_core::types::resource::ResourceRow,
}

/// Flat result emitted by `temper resource update`.
#[derive(Debug, serde::Serialize)]
pub(crate) struct UpdateActionResult {
    pub status: &'static str,
    #[serde(flatten)]
    pub resource: temper_core::types::resource::ResourceRow,
}

/// Result emitted by `temper resource delete`.
///
/// `id` is omitted: `delete_resource` returns `CommandOutput<()>` — the
/// backend does not surface the deleted row, so there is no id in scope
/// at the call site without an extra round-trip.
#[derive(Debug, serde::Serialize)]
pub(crate) struct DeleteActionResult {
    pub status: &'static str,
    pub slug: String,
    pub doc_type: String,
}

/// Result emitted by `temper resource show --edges`. Groups graph edges by
/// direction and routes through `render()` for consistent json|toon output.
#[derive(Debug, serde::Serialize)]
pub(crate) struct EdgesReport {
    pub outgoing: Vec<temper_core::types::graph::GraphEdgeRow>,
    pub incoming: Vec<temper_core::types::graph::GraphEdgeRow>,
}

/// Require a context, returning an error if none specified.
///
/// temper is cloud-only: there are no context directories on disk to
/// check, so a supplied name is trusted directly.
fn require_context(context: Option<&str>) -> Result<String> {
    match context {
        Some(ctx) => Ok(ctx.to_string()),
        None => Err(TemperError::Project(
            "no context specified — use --context <name>".into(),
        )),
    }
}

/// Resolve `--from <path|url>` into a body string via kreuzberg extraction.
///
/// Returns `Some(body)` if `from` is set; `None` if `from` is `None`. Errors
/// when `from` conflicts with `body` or with piped stdin (non-TTY), when the
/// path does not exist, or when extraction fails.
///
/// URL detection: strings with `http://` or `https://` prefix are fetched to a
/// tempfile first, then extracted. Everything else is treated as a local path.
async fn resolve_from_input(
    from: Option<&str>,
    body_flag: Option<&str>,
    stdin_is_tty: bool,
) -> Result<Option<String>> {
    let Some(from) = from else { return Ok(None) };

    if body_flag.is_some() {
        return Err(TemperError::Config(
            "--from cannot be combined with --body".to_string(),
        ));
    }
    if !stdin_is_tty {
        return Err(TemperError::Config(
            "--from cannot be combined with piped stdin".to_string(),
        ));
    }

    let extracted = if from.starts_with("http://") || from.starts_with("https://") {
        let (tmp, _name) = crate::actions::ingest::fetch_url_to_tempfile(from).await?;
        crate::extract::extract_to_markdown(tmp.as_ref()).await?
    } else {
        let path = std::path::Path::new(from);
        if !path.exists() {
            return Err(TemperError::Config(format!(
                "--from path does not exist: {from}"
            )));
        }
        crate::extract::extract_to_markdown(path).await?
    };

    Ok(Some(extracted.content))
}

/// CLI-derived arguments for `create`. Bundles the domain parameters parsed
/// from the `temper resource create` clap subcommand. `config` stays a
/// separate parameter on `create` — it is infrastructure, not CLI-derived
/// domain data. Field ownership mirrors the clap-destructured values to keep
/// the call site free of extra clones.
#[derive(Debug)]
pub struct CreateResourceArgs<'a> {
    pub doc_type: &'a str,
    pub title: &'a str,
    pub context: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub slug: Option<&'a str>,
    pub body_flag: Option<String>,
    pub from: Option<String>,
    pub format: &'a str,
}

/// Create a new resource.
pub fn create(config: &Config, args: CreateResourceArgs<'_>) -> Result<()> {
    let CreateResourceArgs {
        doc_type,
        title,
        context,
        goal,
        mode,
        effort,
        slug,
        body_flag,
        from,
        format,
    } = args;
    use std::io::IsTerminal;

    use temper_core::types::ManagedMeta;

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let ctx = require_context(context)?;

    let stdin_is_tty = std::io::stdin().is_terminal();

    // --from extraction: resolve before body_source so the two are mutually
    // exclusive. The async extract uses a dedicated tokio runtime (does not
    // require a cloud client — kreuzberg operates locally on a file path or
    // fetched tempfile).
    let from_body: Option<String> = if from.is_some() {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
        rt.block_on(resolve_from_input(
            from.as_deref(),
            body_flag.as_deref(),
            stdin_is_tty,
        ))?
    } else {
        None
    };

    // Body resolution — --from wins; fall back to --body flag + stdin pipe.
    let body_opt = if from_body.is_some() {
        from_body
    } else {
        crate::actions::body_source::resolve_body_source(
            body_flag.as_deref(),
            stdin_is_tty,
            std::io::stdin(),
        )?
    };

    // Slug derivation (mode-independent — Concept and Goal skip date prefix).
    let doctype_enum = temper_core::frontmatter::DocType::from_str(doc_type)?;
    let slug_resolved = slug.map(String::from).unwrap_or_else(|| {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let base_slug = vault::slugify(title);
        match doctype_enum {
            // Concept and Goal are identified by name — no date prefix.
            temper_core::frontmatter::DocType::Concept
            | temper_core::frontmatter::DocType::Goal => base_slug,
            // All other doctypes get a date prefix.
            _ => format!("{today}-{base_slug}"),
        }
    });

    // Build the CreateResource cmd. Body-None when no body input; CloudBackend
    // synthesizes `# {title}\n` in its translator for the empty-body case.
    let cmd = temper_core::operations::CreateResource {
        slug: slug_resolved,
        doctype: doc_type.to_string(),
        context: ctx.to_string(),
        title: title.to_string(),
        body: body_opt.filter(|b| !b.is_empty()).map(|content| {
            temper_core::operations::BodyUpdate {
                content,
                content_hash: None,
                chunks_packed: None,
            }
        }),
        managed_meta: ManagedMeta {
            mode: mode.map(String::from),
            effort: effort.map(String::from),
            goal: goal.map(String::from),
            ..ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: temper_core::operations::Surface::CliCloud,
    };

    // Surface-side pre-flight validation — mirrors the hoist of
    // `validate_update_args` for update. Without this, cloud-mode create would
    // skip `validate_create` entirely (CloudBackend has no equivalent), and
    // bad inputs (e.g., --mode plan-or-build whitelist violations) would ship
    // a doomed request to the server. Hoisting here lets the CLI fail-fast
    // before any network call in both modes.
    temper_core::operations::validate_create(&cmd)
        .map_err(|e| TemperError::BadRequest(e.to_string()))?;

    // Acquire the cloud backend + client and dispatch the create.
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.create_resource(cmd))?;

    // Projection refresh: write the new resource to its canonical
    // projection path so the local copy reflects server state at once.
    // Best-effort — a projection write failure must not fail the create.
    let projection_path = match runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &output.value,
    )) {
        Ok(path) => Some(path),
        Err(e) => {
            output::warning(format!("could not write projection file: {e}"));
            None
        }
    };

    // Discovery event for non-Concept/Decision doctypes (Concept and
    // Decision were never emitted pre-Phase 5; preserve that parity).
    if !matches!(
        doctype_enum,
        temper_core::frontmatter::DocType::Concept | temper_core::frontmatter::DocType::Decision
    ) {
        let rel_path = projection_path
            .as_deref()
            .and_then(|p| p.strip_prefix(&config.vault_root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let event = Event::ResourceCreate {
            ts: Local::now().to_rfc3339(),
            doc_type: doc_type.to_string(),
            title: title.to_string(),
            path: rel_path,
            context: ctx.to_string(),
        };
        if let Err(e) = discovery::append_event(&config.state_dir, &event) {
            tracing::warn!("Failed to append discovery event: {e}");
        }
    }

    let result = CreateActionResult {
        status: "ok",
        resource: output.value,
    };
    let fmt = crate::format::OutputFormat::resolve(Some(format));
    let rendered = crate::format::render(&result, fmt)?;
    println!("{rendered}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Cloud-only resource list pipeline
// ---------------------------------------------------------------------------

/// Parameters for the public `show` command, bundled to keep the CLI entry
/// signature compact (and clippy happy).
#[derive(Debug, Clone, Copy)]
pub struct ShowParams<'a> {
    pub doc_type: &'a str,
    pub slug: &'a str,
    pub context: Option<&'a str>,
    pub format: &'a str,
    pub edges: bool,
    pub meta_only: bool,
    pub fields: &'a [String],
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
    pub meta_only: bool,
    pub fields: &'a [String],
}

/// List resources of a given type (unified pipeline for all doc types).
pub fn list(config: &Config, params: ListParams<'_>) -> Result<()> {
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

    if params.meta_only {
        return list_meta_only(config, params);
    }

    use crate::actions::runtime;
    use temper_core::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let fmt = crate::format::OutputFormat::resolve(Some(params.format));
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = params.limit.unwrap_or(20);
    let state_dir = config.state_dir.clone();
    let fields_owned: Vec<String> = params.fields.to_vec();
    let api_params = ResourceListParams {
        doc_type_name: Some(doc_type.clone()),
        context_name: context.clone(),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        ..Default::default()
    };

    // Cloud-only list: a non-blocking staleness pre-flight, then the
    // server query. Any error (network, auth, 4xx/5xx) surfaces as-is —
    // there is no local-scan fallback.
    let response = runtime::with_client(move |client| {
        Box::pin(async move {
            if let Some(ctx) = context.as_deref() {
                crate::projection::warn_if_context_stale(client, &state_dir, ctx).await;
            }
            client
                .resources()
                .list(&api_params)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut envelope = serde_json::to_value(&response)
        .map_err(|e| TemperError::Api(format!("list serialize: {e}")))?;

    if !fields_owned.is_empty() {
        let rows = envelope
            .get_mut("rows")
            .ok_or_else(|| TemperError::Api("response missing `rows` envelope key".into()))?
            .take();
        let filtered_rows =
            temper_core::projection::apply_top_level_filter(rows, &fields_owned, "id")
                .map_err(map_projection_error)?;
        envelope["rows"] = filtered_rows;
    }

    let rendered = crate::format::render(&envelope, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `list --meta-only`: call client.resources().list_meta() and emit
/// the ResourceMetaListResponse shape. Applies the shared top-level
/// projection filter to each row in the envelope when `fields` is
/// non-empty; the envelope keys (`rows`, `total`, `facets`) are
/// preserved untouched.
fn list_meta_only(config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let limit = params.limit.unwrap_or(50);
    let api_params = ResourceListParams {
        doc_type_name: Some(params.doc_type.to_string()),
        context_name: params.context.map(ToString::to_string),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        meta_only: Some(true),
        ..Default::default()
    };
    let format_str = params.format.to_string();
    let fields_owned: Vec<String> = params.fields.to_vec();
    let context_owned = params.context.map(ToString::to_string);
    let state_dir = config.state_dir.clone();

    let response = runtime::with_client(|client| {
        Box::pin(async move {
            if let Some(ctx) = context_owned.as_deref() {
                crate::projection::warn_if_context_stale(client, &state_dir, ctx).await;
            }
            client
                .resources()
                .list_meta(&api_params)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut envelope = serde_json::to_value(&response)
        .map_err(|e| TemperError::Api(format!("meta list serialize: {e}")))?;

    if !fields_owned.is_empty() {
        let rows = envelope
            .get_mut("rows")
            .ok_or_else(|| TemperError::Api("response missing `rows` envelope key".into()))?
            .take();
        let filtered_rows =
            temper_core::projection::apply_top_level_filter(rows, &fields_owned, "resource_id")
                .map_err(map_projection_error)?;
        envelope["rows"] = filtered_rows;
    }

    let fmt = crate::format::OutputFormat::resolve(Some(&format_str));
    let rendered = crate::format::render(&envelope, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Delete a resource.
///
/// temper is cloud-only: the server-side soft-delete is the operation;
/// the projection file is removed afterward as a best-effort tail. The
/// API failure surfaces as an error before any local mutation.
///
/// `force` is forwarded to the backend `DeleteResource` command but does
/// not gate a CLI-side confirmation prompt — cloud delete is
/// non-interactive at the surface.
pub fn delete(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    force: bool,
    format: Option<String>,
) -> Result<()> {
    use temper_core::operations::{DeleteResource, ResourceRef};

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let ctx = require_context(context)?;
    let owner = config.owner_for_context(&ctx);

    let cmd = DeleteResource {
        resource: ResourceRef::scoped(&owner, &ctx, doc_type, slug),
        force,
        origin: temper_core::operations::Surface::CliCloud,
    };

    let (runtime, backend, _client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.delete_resource(cmd))?;

    // Projection refresh: remove the resource's projection file. Best-effort
    // — a removal failure must not fail the (already-committed) delete.
    if let Err(e) =
        crate::projection::remove_resource_file(&config.vault_root, &owner, &ctx, doc_type, slug)
    {
        output::warning(format!("could not remove projection file: {e}"));
    }

    // `delete_resource` returns `CommandOutput<()>` — no row in scope.
    // Emit slug + doc_type from the inputs (Task 9 flat result shape).
    let _ = output;
    let result = DeleteActionResult {
        status: "ok",
        slug: slug.to_string(),
        doc_type: doc_type.to_string(),
    };
    let fmt = crate::format::OutputFormat::resolve(format.as_deref());
    let rendered = crate::format::render(&result, fmt)?;
    println!("{rendered}");

    Ok(())
}

/// Show a resource's content.
pub fn show(config: &Config, params: ShowParams<'_>) -> Result<()> {
    let _ = temper_core::frontmatter::DocType::from_str(params.doc_type)?;

    if params.meta_only {
        return show_meta_only(
            config,
            params.doc_type,
            params.slug,
            params.context,
            params.format,
            params.fields,
        );
    }

    match params.doc_type {
        "task" => crate::commands::task::show(config, params.slug, params.context, params.format),
        "session" => {
            crate::commands::session::show(config, params.slug, params.context, params.format)
        }
        _ => show_generic(
            config,
            params.doc_type,
            params.slug,
            params.context,
            params.format,
        ),
    }?;

    if params.edges {
        let ctx = require_context(params.context)?;
        show_edges(config, &ctx, params.doc_type, params.slug, params.format)?;
    }

    Ok(())
}

/// `show --meta-only`: hit GET /api/resources/{id}/meta and emit the
/// ResourceMetaResponse shape under the chosen format. Applies the
/// shared top-level projection filter when `fields` is non-empty.
///
/// Cloud-only: resolves the resource id via `resolve_by_uri` using
/// the same (owner, context, doc_type, slug) quadruple `show_generic`
/// uses, then calls `get_meta` instead of `content`.
fn show_meta_only(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    fields: &[String],
) -> Result<()> {
    use crate::actions::runtime;

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let config_clone = config.clone();
    let doc_type_inner = doc_type.to_string();
    let slug_inner = slug.to_string();
    let ctx_inner = context.map(str::to_string);
    let fields_inner = fields.to_vec();

    let meta = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = ctx_inner
                .as_deref()
                .ok_or_else(|| {
                    TemperError::Project("no context specified — use --context <name>".into())
                })?
                .to_string();
            let owner = config_clone.owner_for_context(&ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, &ctx, &doc_type_inner, &slug_inner)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let meta = client
                .resources()
                .get_meta(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            Ok(meta)
        })
    })?;

    let value = serde_json::to_value(&meta)
        .map_err(|e| TemperError::Api(format!("meta serialize: {e}")))?;
    let filtered =
        temper_core::projection::apply_top_level_filter(value, &fields_inner, "resource_id")
            .map_err(map_projection_error)?;
    let fmt = crate::format::OutputFormat::resolve(Some(format));
    let rendered = crate::format::render(&filtered, fmt)?;
    println!("{rendered}");
    Ok(())
}

fn map_projection_error(err: temper_core::projection::ProjectionError) -> TemperError {
    use temper_core::projection::ProjectionError;
    match err {
        ProjectionError::DottedPath { hint } => TemperError::Project(format!(
            "--fields supports top-level keys only; use jq for nested projection: {hint}"
        )),
        ProjectionError::EmptyField => {
            TemperError::Project("--fields contained an empty field name".into())
        }
    }
}

/// Show a generic resource (goal, research, concept, decision).
///
/// Cloud-only: resolves the id via `resolve_by_uri`, fetches content,
/// renders it, and writes the canonical projection file (per-resource
/// refresh — best-effort).
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    let slug_s = slug.to_string();
    let context_owned = context.map(str::to_string);
    let format_s = format.to_string();

    let config_clone = config.clone();
    let doc_type_inner = doc_type.to_string();
    let slug_inner = slug_s.clone();
    let ctx_inner = context_owned.clone();

    let (row, body) = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = ctx_inner
                .as_deref()
                .ok_or_else(|| {
                    TemperError::Project("no context specified — use --context <name>".into())
                })?
                .to_string();
            let owner = config_clone.owner_for_context(&ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, &ctx, &doc_type_inner, &slug_inner)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh: write the fetched resource
            // to its canonical projection path. Best-effort — a write
            // failure must not stop `show` from displaying.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!(
                    "could not refresh projection file for '{slug_inner}': {e}"
                ));
            }

            Ok((row, resp.markdown))
        })
    })?;

    let fmt = crate::format::OutputFormat::resolve(Some(&format_s));
    let metadata = serde_json::to_value(&row)
        .map_err(|e| TemperError::Api(format!("metadata serialize: {e}")))?;
    let rendered = crate::format::render_resource_show(&metadata, &body, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Fetch and display edges for a resource via the API.
///
/// Cloud-only: resolves the resource id via `resolve_by_uri` using the
/// same `(owner, context, doc_type, slug)` quadruple the `show` path uses,
/// then fetches and renders the edge list. No manifest access needed.
fn show_edges(
    config: &Config,
    context: &str,
    doc_type: &str,
    slug: &str,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;

    let owner_inner = config.owner_for_context(context);
    let context_inner = context.to_string();
    let doc_type_inner = doc_type.to_string();
    let slug_inner = slug.to_string();

    let resource_id = runtime::with_client(|client| {
        Box::pin(async move {
            let row = client
                .resources()
                .resolve_by_uri(&owner_inner, &context_inner, &doc_type_inner, &slug_inner)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            Ok(*row.id.as_uuid())
        })
    })?;

    let edges: Vec<temper_core::types::graph::GraphEdgeRow> = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .edges(resource_id)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let outgoing: Vec<_> = edges
        .iter()
        .filter(|e| e.direction == "outgoing")
        .cloned()
        .collect();
    let incoming: Vec<_> = edges
        .iter()
        .filter(|e| e.direction == "incoming")
        .cloned()
        .collect();
    let report = EdgesReport { outgoing, incoming };
    let fmt = crate::format::OutputFormat::parse(format);
    let rendered = crate::format::render(&report, fmt)?;
    println!("{rendered}");

    Ok(())
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
    /// Body source flag: `None` (rely on stdin auto-detection — non-empty piped
    /// stdin updates the body; empty implicit stdin means no body update),
    /// `Some("-")` (explicit stdin; errors if empty), or `Some("@<path>")`
    /// (read from file; errors if empty).
    pub body: Option<String>,
    /// Output format: `None` auto-detects from TTY; `Some("json")` or `Some("toon")` explicit.
    pub format: Option<String>,
}

/// Build a partial `ManagedMeta` from update CLI flags. Returns `None` if no
/// managed-meta-mutating flags were passed.
///
/// `title` is a managed-meta scalar (it lands as `temper-title` in
/// frontmatter); B4 added it here so the surface-side dispatch can hand a
/// partial `ManagedMeta` to the backend's `apply_updates` translator without
/// dropping bare `--title` updates on the floor.
fn build_partial_managed_meta_from_args(
    params: &UpdateParams<'_>,
) -> Option<temper_core::types::ManagedMeta> {
    let any_set = params.title.is_some()
        || params.stage.is_some()
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
        title: params.title.map(String::from),
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

/// Typed partial `open_meta` payload built from update CLI list flags.
///
/// Serialized keys are byte-identical to the historical stringly-keyed map:
/// the graph-edge fields carry kebab-case `rename`s. Every field uses
/// `skip_serializing_if` so an all-empty value serializes to `{}` — the
/// `None`-on-empty contract is reconstructed by `build_partial_open_meta_from_args`.
///
/// This is a focused CLI-local struct rather than a reuse of the graph-edge
/// struct in `temper-core` (which omits `tags`/`aliases`/`references` and uses
/// snake_case serialization).
#[derive(Debug, serde::Serialize)]
struct PartialOpenMeta<'a> {
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    tags: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    aliases: &'a [String],
    #[serde(rename = "relates-to", skip_serializing_if = "<[String]>::is_empty")]
    relates_to: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    references: &'a [String],
    #[serde(rename = "depends-on", skip_serializing_if = "<[String]>::is_empty")]
    depends_on: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    extends: &'a [String],
    #[serde(rename = "preceded-by", skip_serializing_if = "<[String]>::is_empty")]
    preceded_by: &'a [String],
    #[serde(rename = "derived-from", skip_serializing_if = "<[String]>::is_empty")]
    derived_from: &'a [String],
}

/// Build a partial `open_meta` JSON object from update CLI list flags. Returns
/// `None` if no open-meta list flags were passed (all vecs empty).
fn build_partial_open_meta_from_args(params: &UpdateParams<'_>) -> Option<serde_json::Value> {
    let partial = PartialOpenMeta {
        tags: params.tags,
        aliases: params.aliases,
        relates_to: params.relates_to,
        references: params.references,
        depends_on: params.depends_on,
        extends: params.extends,
        preceded_by: params.preceded_by,
        derived_from: params.derived_from,
    };
    let value = serde_json::to_value(&partial).ok()?;
    if value.as_object().is_some_and(|o| o.is_empty()) {
        None
    } else {
        Some(value)
    }
}

/// Build a `MoveSpec` from `--type-to` / `--context-to` CLI flags. Returns
/// `None` when neither is set; otherwise returns `Some` with whichever fields
/// were provided. Translates CLI move flags into the `UpdateResource.move_to`
/// operations field.
fn build_move_spec_from_args(
    params: &UpdateParams<'_>,
) -> Option<temper_core::operations::MoveSpec> {
    if params.context_to.is_none() && params.type_to.is_none() {
        return None;
    }
    Some(temper_core::operations::MoveSpec {
        context_to: params.context_to.map(String::from),
        type_to: params.type_to.map(String::from),
    })
}

/// Update a resource's frontmatter fields.
///
/// Surface responsibilities:
///
/// 1. Doctype + type-to validation (clap-side polish; produces a friendlier
///    error than letting `validate_update` surface a `BadRequest`).
/// 2. Per-flag schema validation against `schema::updatable_fields` —
///    rejects bad enum values (e.g. `--stage frobnicate`) with a
///    user-targeted message before the operations layer ever sees them.
/// 3. `--body` flag resolution (stdin/file → `Option<String>`).
/// 4. Build an `UpdateResource` command and dispatch through `build_backend`.
/// 5. Render output: JSON `{"temper-slug": ..., "content_hash": ...}` to
///    stdout — the agent show-edit-cat workflow contract (per CLAUDE.md).
pub fn update(config: &Config, params: &UpdateParams<'_>) -> Result<()> {
    use std::io::IsTerminal;

    use temper_core::operations::{BodyUpdate, ResourceRef, UpdateResource};

    // 1. Resolve current type from --type or --type-from (one is required).
    let current_type = params
        .doc_type
        .or(params.type_from)
        .ok_or_else(|| TemperError::Project("--type or --type-from is required".into()))?;
    let _ = temper_core::frontmatter::DocType::from_str(current_type)?;
    if let Some(tt) = params.type_to {
        let _ = temper_core::frontmatter::DocType::from_str(tt)?;
    }

    // 2. Per-flag schema validation.
    validate_update_args(params, current_type)?;

    // 3. --body resolution.
    let stdin_is_tty = std::io::stdin().is_terminal();
    let resolved_body = crate::actions::body_source::resolve_body_source(
        params.body.as_deref(),
        stdin_is_tty,
        std::io::stdin(),
    )?;

    let ctx = require_context(params.context)?;

    // 4. Build the UpdateResource cmd.
    let cmd = UpdateResource {
        resource: ResourceRef::scoped(
            config.owner_for_context(&ctx),
            &ctx,
            current_type,
            params.slug,
        ),
        body: resolved_body.map(BodyUpdate::new),
        managed_meta: build_partial_managed_meta_from_args(params),
        open_meta: build_partial_open_meta_from_args(params),
        move_to: build_move_spec_from_args(params),
        origin: temper_core::operations::Surface::CliCloud,
    };

    // 5. Acquire the cloud backend + client and dispatch the update.
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.update_resource(cmd))?;

    // 6. Projection refresh: rewrite the affected projection file from
    //    the returned server row. Best-effort — a projection write
    //    failure must not fail the update.
    if let Err(e) = runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &output.value,
    )) {
        output::warning(format!("could not rewrite projection file: {e}"));
    }

    // 7. Emit the flat UpdateActionResult to stdout (Task 9: replaces the
    //    bespoke { "temper-slug", "content_hash" } shape).
    let result = UpdateActionResult {
        status: "ok",
        resource: output.value,
    };
    let fmt = crate::format::OutputFormat::resolve(params.format.as_deref());
    let rendered = crate::format::render(&result, fmt)?;
    println!("{rendered}");

    Ok(())
}

/// Per-flag schema validation for `update`. Lifted from the pre-B4 surface
/// code so the friendlier per-flag error messages survive the migration.
/// Only validates scalar managed-meta flags; array fields and `title` (a
/// base-schema field valid on all doctypes) are skipped.
fn validate_update_args(params: &UpdateParams<'_>, current_type: &str) -> Result<()> {
    // Build list of scalar field updates: (frontmatter_key, value) — a
    // direct lift of the pre-B4 inline assembly so the validation loop
    // semantics are unchanged.
    let scalar_updates: Vec<(&str, String)> = [
        ("temper-title", params.title.map(String::from)),
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

    if scalar_updates.is_empty() {
        return Ok(());
    }

    let schema_fields = schema::updatable_fields(current_type)?;

    // Base fields valid on all types (from base.schema.json).
    const BASE_FIELDS: &[&str] = &["temper-title"];

    for (field_name, value) in &scalar_updates {
        if BASE_FIELDS.contains(field_name) {
            continue;
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

    Ok(())
}

#[cfg(test)]
mod build_helpers_tests {
    use super::*;

    /// Construct an `UpdateParams` with all optional/list fields defaulted.
    /// Tests override only the fields they exercise.
    fn empty_update_params<'a>(slug: &'a str) -> UpdateParams<'a> {
        UpdateParams {
            slug,
            doc_type: None,
            type_from: None,
            type_to: None,
            context: None,
            context_to: None,
            title: None,
            tags: &[],
            aliases: &[],
            relates_to: &[],
            references: &[],
            depends_on: &[],
            extends: &[],
            preceded_by: &[],
            derived_from: &[],
            stage: None,
            mode: None,
            effort: None,
            goal: None,
            seq: None,
            branch: None,
            pr: None,
            status: None,
            body: None,
            format: None,
        }
    }

    #[test]
    fn build_move_spec_returns_none_when_both_flags_unset() {
        let params = empty_update_params("foo");
        assert!(build_move_spec_from_args(&params).is_none());
    }

    #[test]
    fn build_move_spec_returns_some_with_only_context_to_when_only_context_to_set() {
        let mut params = empty_update_params("foo");
        params.context_to = Some("temper");
        let spec = build_move_spec_from_args(&params).expect("expected Some");
        assert_eq!(spec.context_to, Some("temper".to_string()));
        assert_eq!(spec.type_to, None);
    }

    #[test]
    fn build_move_spec_returns_some_with_both_when_both_set() {
        let mut params = empty_update_params("foo");
        params.context_to = Some("temper");
        params.type_to = Some("concept");
        let spec = build_move_spec_from_args(&params).expect("expected Some");
        assert_eq!(spec.context_to, Some("temper".to_string()));
        assert_eq!(spec.type_to, Some("concept".to_string()));
    }

    /// `title` is a managed-meta field (`temper-title`) and must propagate
    /// through `build_partial_managed_meta_from_args` so the B4 surface-side
    /// dispatch can hand a partial `ManagedMeta` (carrying `title`) to the
    /// backend's `apply_updates` translator. Pre-B4 the helper omitted
    /// `title`; this test guards against re-introducing that gap.
    #[test]
    fn build_partial_managed_meta_from_args_includes_title_when_set() {
        let mut params = empty_update_params("foo");
        params.title = Some("Renamed Resource");
        let mm = build_partial_managed_meta_from_args(&params).expect("expected Some");
        assert_eq!(mm.title.as_deref(), Some("Renamed Resource"));
    }

    /// Regression guard: a bare `--title` (no other managed flags) must still
    /// trip the `any_set` short-circuit so the helper returns `Some(..)`. If
    /// `any_set` were to omit `title`, callers passing only `--title` would
    /// see `None` and the title update would silently no-op.
    #[test]
    fn build_partial_managed_meta_from_args_returns_some_when_only_title_set() {
        let mut params = empty_update_params("foo");
        params.title = Some("Solo title");
        assert!(
            build_partial_managed_meta_from_args(&params).is_some(),
            "title-only must trip any_set"
        );
    }
}

#[cfg(test)]
mod action_result_tests {
    use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
    use temper_core::types::resource::ResourceRow;

    use super::{CreateActionResult, DeleteActionResult, UpdateActionResult};

    /// Build a minimal `ResourceRow` fixture for action result tests.
    pub(super) fn make_resource_row(
        slug: &str,
        doc_type: &str,
        title: &str,
        context: &str,
    ) -> ResourceRow {
        ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: ContextId(uuid::Uuid::nil()),
            kb_doc_type_id: DocTypeId(uuid::Uuid::nil()),
            origin_uri: "test://origin".to_string(),
            title: title.to_string(),
            slug: Some(slug.to_string()),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: context.to_string(),
            doc_type_name: doc_type.to_string(),
            owner_handle: "@me".to_string(),
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        }
    }

    /// Task 9: `CreateActionResult` flattens `ResourceRow` — all wire-type
    /// fields appear at the top level alongside `status`. The old per-doctype
    /// `temper-slug` / `temper-title` keys must not appear.
    #[test]
    fn render_create_action_result_json_is_flat() {
        let row = make_resource_row("2026-05-14-test", "task", "Test Task", "temper");
        let result = CreateActionResult {
            status: "ok",
            resource: row,
        };
        let out =
            crate::format::render(&result, crate::format::OutputFormat::Json).expect("json render");

        // status and flattened wire fields at top level.
        assert!(out.contains("\"status\": \"ok\""), "status missing: {out}");
        assert!(out.contains("\"slug\""), "slug missing: {out}");
        assert!(out.contains("\"title\""), "title missing: {out}");
        assert!(
            out.contains("\"context_name\""),
            "context_name missing: {out}"
        );

        // Old per-doctype keys must not appear.
        assert!(
            !out.contains("temper-slug"),
            "legacy temper-slug key must not appear: {out}"
        );
        assert!(
            !out.contains("temper-title"),
            "legacy temper-title key must not appear: {out}"
        );
        assert!(
            !out.contains("temper-context"),
            "legacy temper-context key must not appear: {out}"
        );
    }

    /// Flat shape works for all doctypes — research previously used a
    /// distinct `project` key; now `context_name` is the wire field.
    #[test]
    fn render_create_action_result_research_uses_wire_context_name() {
        let row = make_resource_row(
            "2026-05-14-my-research",
            "research",
            "My Research",
            "temper",
        );
        let result = CreateActionResult {
            status: "ok",
            resource: row,
        };
        let out =
            crate::format::render(&result, crate::format::OutputFormat::Json).expect("json render");

        // Wire field name, not legacy `project`.
        assert!(
            out.contains("\"context_name\""),
            "context_name missing: {out}"
        );
        assert!(
            !out.contains("\"project\""),
            "legacy project key must not appear: {out}"
        );
    }

    /// `UpdateActionResult` has the same flat shape as `CreateActionResult`.
    #[test]
    fn render_update_action_result_json_is_flat() {
        let mut row = make_resource_row("my-task", "task", "My Task", "temper");
        row.body_hash = Some("sha256:abc".to_string());
        let result = UpdateActionResult {
            status: "ok",
            resource: row,
        };
        let out =
            crate::format::render(&result, crate::format::OutputFormat::Json).expect("json render");

        assert!(out.contains("\"status\": \"ok\""), "status missing: {out}");
        assert!(out.contains("\"slug\""), "slug missing: {out}");
        // body_hash is now visible (was hidden in the old { temper-slug, content_hash } shape).
        assert!(
            out.contains("body_hash"),
            "body_hash should appear in wire passthrough: {out}"
        );
        // Old bespoke key must not appear.
        assert!(
            !out.contains("content_hash"),
            "legacy content_hash key must not appear as a separate top-level field: {out}"
        );
    }

    /// `DeleteActionResult` emits `{ status, slug, doc_type }`.
    #[test]
    fn render_delete_action_result_json_includes_slug_and_doc_type() {
        let result = DeleteActionResult {
            status: "ok",
            slug: "test-slug".to_string(),
            doc_type: "task".to_string(),
        };
        let out =
            crate::format::render(&result, crate::format::OutputFormat::Json).expect("json render");

        assert!(out.contains("\"status\": \"ok\""), "status missing: {out}");
        assert!(
            out.contains("\"slug\": \"test-slug\""),
            "slug missing: {out}"
        );
        assert!(
            out.contains("\"doc_type\": \"task\""),
            "doc_type missing: {out}"
        );
    }
}

#[cfg(test)]
mod from_flag_tests {
    use super::*;

    #[tokio::test]
    async fn from_and_body_are_mutually_exclusive() {
        // resolve_from_input errors when both --from and --body are provided.
        let err = resolve_from_input(Some("/tmp/x.md"), Some("@body.md"), true)
            .await
            .expect_err("should error on mutex");
        assert!(
            format!("{err}").contains("--from cannot be combined with --body"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn from_and_piped_stdin_are_mutually_exclusive() {
        // resolve_from_input errors when --from is set and stdin is non-TTY.
        let err = resolve_from_input(Some("/tmp/x.md"), None, false)
            .await
            .expect_err("should error on non-TTY stdin");
        assert!(
            format!("{err}").contains("--from cannot be combined with piped stdin"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn from_path_does_not_exist_errors() {
        // resolve_from_input errors when the path doesn't exist.
        let err = resolve_from_input(Some("/tmp/definitely_does_not_exist_ch7.md"), None, true)
            .await
            .expect_err("should error on missing path");
        assert!(
            format!("{err}").contains("--from path does not exist"),
            "got: {err}"
        );
    }
}

#[cfg(test)]
mod resource_list_render_tests {
    use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
    use temper_core::types::resource::ResourceRow;

    /// Task 7: verify that `render()` passthrough includes internal wire fields
    /// like `body_hash` that the old `row_to_frontmatter_value` + `render_server_rows`
    /// path deliberately dropped. This is the canary for the breaking change.
    #[test]
    fn render_resource_list_json_passes_wire_type_with_internals() {
        let rows: Vec<ResourceRow> = vec![ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: ContextId(uuid::Uuid::nil()),
            kb_doc_type_id: DocTypeId(uuid::Uuid::nil()),
            origin_uri: "test://origin".to_string(),
            title: "Test Resource".to_string(),
            slug: Some("test-resource".to_string()),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            context_name: "temper".to_string(),
            doc_type_name: "research".to_string(),
            owner_handle: "@me".to_string(),
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: Some("abc123deadbeef".to_string()),
            managed_hash: None,
            open_hash: None,
        }];

        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");

        // The whole point of Task 7 is that internal fields are now visible.
        // body_hash is the canary; if the old re-shaping survives anywhere, this fails.
        assert!(
            out.contains("body_hash") || out.contains("\"body_hash\""),
            "body_hash should appear in passthrough JSON: {out}"
        );
        // Old frontmatter keys must NOT appear — they were the re-shaped output.
        assert!(
            !out.contains("temper-slug"),
            "re-shaped temper-slug key must not appear in wire passthrough: {out}"
        );
        assert!(
            !out.contains("temper-title"),
            "re-shaped temper-title key must not appear in wire passthrough: {out}"
        );
        // The actual wire field names should be present.
        assert!(
            out.contains("\"title\""),
            "wire field 'title' missing: {out}"
        );
        assert!(out.contains("\"slug\""), "wire field 'slug' missing: {out}");
    }
}

/// Tests for the `EdgesReport` struct and its render path.
#[cfg(test)]
mod edges_report_tests {
    use super::EdgesReport;
    use temper_core::types::graph::{EdgeKind, GraphEdgeRow, Polarity};

    fn make_edge(direction: &str, label: &str) -> GraphEdgeRow {
        GraphEdgeRow {
            edge_id: uuid::Uuid::nil(),
            peer_resource_id: uuid::Uuid::nil(),
            peer_title: "Peer Title".to_string(),
            peer_slug: "peer-slug".to_string(),
            edge_kind: EdgeKind::Express,
            polarity: Polarity::Forward,
            label: label.to_string(),
            direction: direction.to_string(),
            weight: 1.0,
            created: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn render_edges_report_json_passthrough() {
        let report = EdgesReport {
            outgoing: vec![make_edge("outgoing", "depends_on")],
            incoming: vec![make_edge("incoming", "blocks")],
        };
        let out =
            crate::format::render(&report, crate::format::OutputFormat::Json).expect("json render");
        assert!(
            out.contains("\"outgoing\""),
            "json should have outgoing key: {out}"
        );
        assert!(
            out.contains("\"incoming\""),
            "json should have incoming key: {out}"
        );
        assert!(
            out.contains("\"depends_on\""),
            "outgoing label should appear: {out}"
        );
        assert!(
            out.contains("\"blocks\""),
            "incoming label should appear: {out}"
        );
    }

    #[test]
    fn render_edges_report_empty_emits_empty_arrays() {
        let report = EdgesReport {
            outgoing: vec![],
            incoming: vec![],
        };
        let out =
            crate::format::render(&report, crate::format::OutputFormat::Json).expect("json render");
        assert!(
            out.contains("\"outgoing\": []"),
            "empty outgoing should be []: {out}"
        );
        assert!(
            out.contains("\"incoming\": []"),
            "empty incoming should be []: {out}"
        );
    }
}

/// Tests for the `session::show` migration — verifies that the render path
/// uses `render_resource_show` and produces the correct json|toon shapes
/// given a session-shaped metadata fixture.
#[cfg(test)]
mod session_show_render_tests {
    #[test]
    fn render_resource_show_session_json_includes_content_key() {
        // Simulate the metadata shape emitted by `session::show` after
        // migrating to `render_resource_show`. The session row serializes to
        // a `ResourceRow`-shaped value; the body becomes `content`.
        let metadata = serde_json::json!({
            "slug": "2026-05-26-daily-standup",
            "title": "Daily Standup",
            "doc_type_name": "session",
            "context_name": "temper",
        });
        let body = "# Daily Standup\n\nToday's notes.\n";
        let out =
            crate::format::render_resource_show(&metadata, body, crate::format::OutputFormat::Json)
                .expect("json render");
        assert!(
            out.contains("\"content\""),
            "json composite must have content key: {out}"
        );
        assert!(
            out.contains("Today's notes"),
            "json must embed the body: {out}"
        );
        assert!(
            out.contains("\"doc_type_name\""),
            "metadata fields must be preserved: {out}"
        );
    }

    #[test]
    fn render_resource_show_session_toon_emits_frontmatter_then_body() {
        let metadata = serde_json::json!({
            "slug": "2026-05-26-daily-standup",
            "title": "Daily Standup",
        });
        let body = "# Daily Standup\n\nToday's notes.\n";
        let out =
            crate::format::render_resource_show(&metadata, body, crate::format::OutputFormat::Toon)
                .expect("toon render");
        assert!(
            out.starts_with("---\n"),
            "toon must open with frontmatter: {out}"
        );
        assert!(
            out.contains("Daily Standup"),
            "toon must include body: {out}"
        );
    }
}

#[cfg(test)]
mod resource_show_render_tests {
    #[test]
    fn render_resource_show_toon_emits_frontmatter_then_body() {
        let metadata = serde_json::json!({
            "temper-title": "Hello",
            "temper-slug": "hello",
        });
        let body = "# Hello\n\nBody text.\n";
        let out =
            crate::format::render_resource_show(&metadata, body, crate::format::OutputFormat::Toon)
                .expect("toon render");
        assert!(
            out.starts_with("---\n"),
            "toon should start with frontmatter fence: {out}"
        );
        assert!(out.contains("# Hello"), "toon body missing: {out}");
        assert!(
            out.contains("temper-title"),
            "frontmatter title missing: {out}"
        );
    }

    #[test]
    fn render_resource_show_json_emits_composite() {
        let metadata = serde_json::json!({
            "slug": "hello",
            "title": "Hello",
        });
        let body = "# Hello\n\nBody text.\n";
        let out =
            crate::format::render_resource_show(&metadata, body, crate::format::OutputFormat::Json)
                .expect("json render");
        assert!(
            out.contains("\"content\""),
            "json should have content key: {out}"
        );
        assert!(out.contains("# Hello"), "body should be embedded: {out}");
        assert!(
            out.contains("\"slug\""),
            "metadata should be preserved: {out}"
        );
    }
}

#[cfg(test)]
mod list_meta_only_tests {
    use temper_core::projection::apply_top_level_filter;

    #[test]
    fn list_meta_filter_applies_per_row_and_preserves_envelope() {
        // Build a stub ResourceMetaListResponse-shaped JSON
        let envelope = serde_json::json!({
            "rows": [
                {
                    "resource_id": "11111111-1111-1111-1111-111111111111",
                    "managed_meta": {"stage": "in-progress"},
                    "open_meta": {"tags": []},
                    "managed_hash": "sha256:a",
                    "open_hash": "sha256:b"
                },
                {
                    "resource_id": "22222222-2222-2222-2222-222222222222",
                    "managed_meta": {"stage": "done"},
                    "open_meta": null,
                    "managed_hash": "sha256:c",
                    "open_hash": "sha256:d"
                }
            ],
            "total": 2,
            "facets": {"doc_type": {"task": 2}}
        });

        // Filter the rows array (the action layer will apply the filter
        // to envelope.rows specifically, not to the whole envelope).
        let rows = envelope.get("rows").cloned().expect("rows");
        let filtered_rows =
            apply_top_level_filter(rows, &["managed_meta".to_string()], "resource_id")
                .expect("filter");

        // Each row should have only resource_id + managed_meta
        let arr = filtered_rows.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for row in arr {
            assert!(row.get("resource_id").is_some(), "anchor missing in {row}");
            assert!(
                row.get("managed_meta").is_some(),
                "managed_meta missing in {row}"
            );
            assert!(
                row.get("open_meta").is_none(),
                "open_meta should be dropped"
            );
            assert!(row.get("managed_hash").is_none(), "hash should be dropped");
        }
    }
}

#[cfg(test)]
mod show_meta_only_tests {
    use temper_core::projection::apply_top_level_filter;
    use temper_core::types::managed_meta::{ManagedMeta, ResourceMetaResponse};

    fn fake_meta_response() -> ResourceMetaResponse {
        ResourceMetaResponse {
            resource_id: temper_core::types::ResourceId::from(uuid::Uuid::nil()),
            managed_meta: Some(ManagedMeta {
                title: Some("test".to_string()),
                ..Default::default()
            }),
            open_meta: Some(serde_json::json!({"tags": ["x"]})),
            managed_hash: "sha256:test".to_string(),
            open_hash: "sha256:test".to_string(),
        }
    }

    #[test]
    fn show_meta_only_fields_filter_preserves_anchor_and_managed_meta_only() {
        let response = fake_meta_response();
        let value = serde_json::to_value(&response).expect("serialize");
        let filtered = apply_top_level_filter(value, &["managed_meta".to_string()], "resource_id")
            .expect("filter");
        assert!(filtered.get("resource_id").is_some(), "anchor missing");
        assert!(
            filtered.get("managed_meta").is_some(),
            "managed_meta missing"
        );
        assert!(
            filtered.get("open_meta").is_none(),
            "open_meta should be filtered out"
        );
        assert!(
            filtered.get("managed_hash").is_none(),
            "managed_hash should be filtered out"
        );
    }

    #[test]
    fn show_meta_only_no_fields_returns_full_response() {
        let response = fake_meta_response();
        let value = serde_json::to_value(&response).expect("serialize");
        let unfiltered = apply_top_level_filter(value.clone(), &[], "resource_id").expect("filter");
        assert_eq!(unfiltered, value);
    }
}
