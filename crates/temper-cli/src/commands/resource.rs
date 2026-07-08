use chrono::Local;
use temper_workflow::schema;

use crate::config::Config;
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
    pub resource: temper_workflow::types::resource::ResourceRow,
}

/// Flat result emitted by `temper resource update`.
#[derive(Debug, serde::Serialize)]
pub(crate) struct UpdateActionResult {
    pub status: &'static str,
    #[serde(flatten)]
    pub resource: temper_workflow::types::resource::ResourceRow,
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
    pub outgoing: Vec<temper_workflow::types::graph::GraphEdgeRow>,
    pub incoming: Vec<temper_workflow::types::graph::GraphEdgeRow>,
}

/// Insert a derived `ref` key (the decorated, self-resolving identifier)
/// into a serialized resource row, computed from its id + `title`. The
/// `ref` is render-time only — never persisted, never on the wire type.
/// Reads the anchor id from `id` (ResourceRow) OR `resource_id`
/// (UnifiedSearchResultRow). No-op if the id is absent or unparseable.
///
/// Also injects `context_ref` — the decorated home-context ref
/// (`{context_owner_ref}/{context_slug}`) — when both fields are present
/// on the row. This lets agents and UIs address the resource's home
/// context without a second round-trip.
pub(crate) fn inject_ref(row: &mut serde_json::Value) {
    let id = row
        .get("id")
        .or_else(|| row.get("resource_id"))
        .and_then(|v| v.as_str());
    let Some(id) = id else { return };
    let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("");
    if let Ok(uuid) = uuid::Uuid::parse_str(id) {
        let decorated = temper_workflow::operations::decorated_ref(
            title,
            temper_core::types::ids::ResourceId(uuid),
        );
        if let Some(obj) = row.as_object_mut() {
            obj.insert("ref".to_string(), serde_json::Value::String(decorated));

            // Inject context_ref alongside ref when the row carries the raw ingredients.
            let ctx_owner_ref = obj
                .get("context_owner_ref")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let ctx_slug = obj
                .get("context_slug")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            if let (Some(owner_ref), Some(slug)) = (ctx_owner_ref, ctx_slug) {
                let context_ref = format!("{owner_ref}/{slug}");
                obj.insert(
                    "context_ref".to_string(),
                    serde_json::Value::String(context_ref),
                );
            }
        }
    }
}

/// Insert a derived `ref` key into a serialized context row
/// (`ContextRow` / `ContextRowWithCounts`), computed from `owner_ref` + `slug`.
/// The `ref` is render-time only — never persisted, never on the wire type.
/// No-op if `owner_ref` or `slug` are absent from the row.
pub(crate) fn inject_context_ref(row: &mut serde_json::Value) {
    if let Some(obj) = row.as_object_mut() {
        let owner_ref = obj
            .get("owner_ref")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let slug = obj.get("slug").and_then(|v| v.as_str()).map(str::to_owned);
        if let (Some(owner_ref), Some(slug)) = (owner_ref, slug) {
            let decorated = format!("{owner_ref}/{slug}");
            obj.insert("ref".to_string(), serde_json::Value::String(decorated));
        }
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
    /// Cognitive-map ref to home the resource in. Mutually exclusive with
    /// `context`; the surface enforces exactly-one.
    pub cogmap: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub slug: Option<&'a str>,
    /// Open (caller-defined) frontmatter as a raw `--open-meta` JSON object
    /// string. Parsed + validated (must be a JSON object) by `parse_open_meta_flag`.
    pub open_meta: Option<&'a str>,
    /// Goal link target ref (`--goal`). When `Some`, resolved via `parse_ref` and
    /// projected to a live `advances`→goal edge on create.
    pub goal: Option<&'a str>,
    /// Session→task link target slug (session only). When `Some`, after the
    /// session is created a session→task `advances` relationship is asserted.
    pub task: Option<&'a str>,
    pub body_flag: Option<String>,
    pub from: Option<String>,
    /// Provenance source refs (`--sources`) — resolved to `ProvenanceSource::Resource`
    /// via `parse_ref` and attached to the body block. Requires a body.
    pub sources: Vec<String>,
    pub format: crate::format::OutputFormat,
    /// Per-act correlation + authorship for the create act (from `--invocation`/`--confidence`/…).
    pub act: temper_core::types::ActInput,
}

/// Resolve `--sources` values to `ProvenanceSource`s: an http/https URL → `Remote` (external
/// source), any other value → a ref (UUID or decorated) → `Resource`. A value that is neither a URL
/// nor a parseable ref is a hard error — never a silent drop (parse-don't-validate / escalate). The
/// classifier is shared with the MCP surface so both classify identically.
fn resolve_provenance_sources(
    refs: &[String],
) -> Result<Vec<temper_core::types::provenance::ProvenanceSource>> {
    refs.iter()
        .map(|r| temper_workflow::operations::resolve_provenance_source(r))
        .collect()
}

/// Create a new resource.
pub fn create(config: &Config, args: CreateResourceArgs<'_>) -> Result<()> {
    let CreateResourceArgs {
        doc_type,
        title,
        context,
        cogmap,
        mode,
        effort,
        slug,
        open_meta,
        goal,
        task,
        body_flag,
        from,
        sources,
        format,
        act,
    } = args;
    use std::io::IsTerminal;

    use temper_workflow::types::ManagedMeta;

    // Open tail (Task A2): no client-side doctype fail-fast here — the
    // server gate (`validate_create` / `validate_doctype`) governs, and an
    // unrecognized doctype is a legitimate free string, not a client error.

    // Fail-fast: --task linking is only valid for sessions. Reject before any
    // create round-trip (mirrors the validate_create fail-fast hoist below).
    if task.is_some() && doc_type != "session" {
        return Err(TemperError::BadRequest(format!(
            "--task linking is only supported for --type session (got --type {doc_type})"
        )));
    }

    // Home resolution — exactly one of --context / --cogmap. The home choice is
    // a `HomeAnchor` enum (never a placeholder id plus a flag): a context home
    // carries a placeholder id (the real ref is threaded via the cloud backend's
    // `context_ref`), a cogmap home carries the resolved `CogmapId`.
    let (home, ctx) = match (context, cogmap) {
        (Some(_), Some(_)) => {
            return Err(TemperError::BadRequest(
                "--context and --cogmap are mutually exclusive; specify exactly one home".into(),
            ));
        }
        (None, None) => {
            return Err(TemperError::Project(
                "no home specified — use --context <ref> (e.g. @me/temper) or --cogmap <ref>"
                    .into(),
            ));
        }
        (Some(context), None) => (
            temper_core::types::home::HomeAnchor::Context(temper_core::types::ids::ContextId::new()),
            context.to_string(),
        ),
        (None, Some(cogmap)) => {
            // Trailing-UUID-only resolution (no server lookup); the slug half is
            // parsed off and ignored.
            let id = temper_workflow::operations::parse_ref(cogmap)?.0;
            (
                temper_core::types::home::HomeAnchor::Cogmap(
                    temper_core::types::ids::CogmapId::from(id),
                ),
                cogmap.to_string(),
            )
        }
    };

    let stdin_is_tty = std::io::stdin().is_terminal();

    // Body resolution — --from wins; fall back to --body flag + stdin pipe.
    let body_opt = resolve_create_body(from.as_deref(), body_flag.as_deref(), stdin_is_tty)?;

    // Slug derivation (mode-independent — Concept and Goal skip date prefix).
    // An unrecognized (open-tail) doctype has no variant, so it falls back to
    // the date-prefixed `_` catch-all inside `derive_create_slug`, the same
    // as any known non-Concept/Goal doctype.
    //
    // The slug is ALWAYS the title-derived value: slug is §7-dissolved (never
    // stored; addressing is trailing-UUID-only), so an explicit `--slug` cannot
    // be honored. Rather than silently discard a differing override (issue #307
    // Bug 2), reject it — a matching value is a harmless no-op.
    let doctype_enum = temper_workflow::frontmatter::DocType::from_str(doc_type).ok();
    let slug_resolved = derive_create_slug(None, title, doctype_enum);
    if let Some(explicit) = slug {
        if explicit != slug_resolved {
            return Err(TemperError::BadRequest(format!(
                "--slug '{explicit}' cannot be honored: the slug is derived from the title \
                 ('{slug_resolved}') and addressing is trailing-UUID-only, so an override is \
                 not stored. Omit --slug."
            )));
        }
    }

    // Parse the optional --open-meta JSON object (the free-form open tier).
    let open_meta_value = open_meta.map(parse_open_meta_flag).transpose()?;

    // Build the CreateResource cmd. Body-None when no body input; CloudBackend
    // synthesizes `# {title}\n` in its translator for the empty-body case.
    // For a context home, `home` carries a placeholder id and the actual context
    // ref (`ctx`) is threaded through `CloudBackend.context_ref` to
    // `cmd_to_ingest_payload`; for a cogmap home, `home` carries the resolved
    // `CogmapId` and the translator sends `home_cogmap_id` with an empty
    // `context_ref`.
    // Resolve --sources refs → provenance records for the body block. A ref that fails to
    // parse is a hard error (escalate, never silently drop); sources without a body have
    // nothing to attribute.
    let resolved_sources = resolve_provenance_sources(&sources)?;
    let body_content = body_opt.filter(|b| !b.is_empty());
    if !resolved_sources.is_empty() && body_content.is_none() {
        return Err(TemperError::BadRequest(
            "--sources requires a body update; add --body/--from or pipe content".into(),
        ));
    }

    // Resolve --goal ref → goal resource id (trailing-UUID-only, like `edge assert`); the server
    // projects the live `advances`→goal edge after create. An unparseable ref is a hard error.
    let goal_resolved = goal
        .map(temper_workflow::operations::parse_ref)
        .transpose()?;

    let cmd = temper_workflow::operations::CreateResource {
        slug: slug_resolved,
        doctype: doc_type.to_string(),
        home,
        title: title.to_string(),
        body: body_content.map(|content| temper_workflow::operations::BodyUpdate {
            content,
            content_hash: None,
            chunks_packed: None,
            sources: resolved_sources,
            // Create writes a single new body block; per-block addressing is update-only.
            content_block: None,
        }),
        managed_meta: ManagedMeta {
            mode: mode.map(String::from),
            effort: effort.map(String::from),
            ..ManagedMeta::default()
        },
        open_meta: open_meta_value,
        goal: goal_resolved,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        act: act.into_act_context()?,
        origin: temper_workflow::operations::Surface::CliCloud,
    };

    // Surface-side pre-flight validation — mirrors the hoist of
    // `validate_update_args` for update. Without this, cloud-mode create would
    // skip `validate_create` entirely (CloudBackend has no equivalent), and
    // bad inputs (e.g., --mode plan-or-build whitelist violations) would ship
    // a doomed request to the server. Hoisting here lets the CLI fail-fast
    // before any network call in both modes.
    temper_workflow::operations::validate_create(&cmd)
        .map_err(|e| TemperError::BadRequest(e.to_string()))?;

    // Acquire the cloud backend + client and dispatch the create.
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.create_resource(cmd))?;

    // Projection refresh: write the new resource to its canonical
    // projection path so the local copy reflects server state at once.
    // Best-effort — a projection write failure must not fail the create.
    if let Err(e) = runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &output.value,
    )) {
        output::warning(format!("could not write projection file: {e}"));
    }

    // Session→task linking. Only reached for sessions (validated fail-fast
    // above). The session resource is already created; the link is a best-
    // effort tail — an unknown task warns and skips rather than failing the
    // (already-committed) create.
    if let Some(task_slug) = task {
        link_session_to_task(config, &runtime, &client, &ctx, output.value.id, task_slug);
    }

    let result = CreateActionResult {
        status: "ok",
        resource: output.value,
    };
    let rendered = crate::format::render(&result, format)?;
    println!("{rendered}");
    Ok(())
}

/// Resolve the create body: `--from <path|url>` wins (extracted via a
/// dedicated tokio runtime — kreuzberg operates locally), falling back to
/// the `--body` flag plus stdin pipe. `--from` is mutually exclusive with
/// `--body`/piped stdin; that conflict is enforced by `resolve_from_input`.
fn resolve_create_body(
    from: Option<&str>,
    body_flag: Option<&str>,
    stdin_is_tty: bool,
) -> Result<Option<String>> {
    let from_body: Option<String> = if from.is_some() {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
        rt.block_on(resolve_from_input(from, body_flag, stdin_is_tty))?
    } else {
        None
    };

    if from_body.is_some() {
        Ok(from_body)
    } else {
        crate::actions::body_source::resolve_body_source(
            body_flag,
            stdin_is_tty,
            std::io::stdin(),
            crate::actions::body_source::stdin_has_input_within,
        )
    }
}

/// Derive a resource slug: an explicit `--slug` is used verbatim; otherwise
/// derive from the title, date-prefixing every doctype except Concept and
/// Goal (which are identified by name). `doctype` is `None` for an
/// unrecognized (open-tail) label, which falls into the date-prefixed
/// catch-all alongside every other non-Concept/Goal doctype.
fn derive_create_slug(
    slug: Option<&str>,
    title: &str,
    doctype: Option<temper_workflow::frontmatter::DocType>,
) -> String {
    slug.map(String::from).unwrap_or_else(|| {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let base_slug = vault::slugify(title);
        match doctype {
            // Concept and Goal are identified by name — no date prefix.
            Some(temper_workflow::frontmatter::DocType::Concept)
            | Some(temper_workflow::frontmatter::DocType::Goal) => base_slug,
            // Every other doctype (known or open-tail/unrecognized) gets a date prefix.
            _ => format!("{today}-{base_slug}"),
        }
    })
}

/// Assert the session→task `advances` link after a session create.
///
/// Best-effort: the session is already committed, so every failure mode
/// (unknown/ambiguous/errored task lookup, or a failed assert) warns and
/// returns rather than failing the create. `find_task` owns its own runtime
/// via `with_client`, so it is called outside `runtime`.
fn link_session_to_task(
    config: &Config,
    runtime: &tokio::runtime::Runtime,
    client: &temper_client::TemperClient,
    ctx: &str,
    session_id: temper_core::types::ids::ResourceId,
    task_slug: &str,
) {
    match crate::actions::task::find_task(config, task_slug, Some(ctx)) {
        Ok(Some(task_info)) => {
            use temper_core::types::graph::{EdgeKind, Polarity};
            use temper_core::types::relationship_requests::AssertRelationshipRequest;

            // Edge addressing is id-based now: `find_task` carried the task's
            // resource id off the listing row, so the link asserts by that
            // held id directly — no slug→id round-trip.
            let result = runtime.block_on(async {
                let req = AssertRelationshipRequest {
                    source: session_id,
                    target: task_info.id,
                    edge_kind: EdgeKind::LeadsTo,
                    polarity: Polarity::Forward,
                    label: "advances".to_string(),
                    weight: 1.0,
                    // System-driven link (not a caller-authored act): empty act context.
                    act: Default::default(),
                };
                client
                    .relationships()
                    .assert(&req)
                    .await
                    .map_err(crate::commands::client_err)
            });
            match result {
                Ok(_) => output::success(format!("Linked session → task {}", task_info.slug)),
                Err(e) => tracing::warn!(
                    task = task_slug,
                    error = %e,
                    "session→task assert failed; session created without link"
                ),
            }
        }
        Ok(None) => {
            tracing::warn!(
                task = task_slug,
                "task not found for session link; skipping relationship assert"
            );
        }
        Err(e) => {
            tracing::warn!(
                task = task_slug,
                error = %e,
                "task lookup failed for session link; skipping relationship assert"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cloud-only resource list pipeline
// ---------------------------------------------------------------------------

/// Parameters for the public `show` command, bundled to keep the CLI entry
/// signature compact (and clippy happy).
#[derive(Debug, Clone, Copy)]
pub struct ShowParams<'a> {
    pub r#ref: &'a str,
    pub format: crate::format::OutputFormat,
    pub edges: bool,
    pub provenance: bool,
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
    pub format: crate::format::OutputFormat,
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
    use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let fmt = params.format;
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = params.limit.unwrap_or(20);
    let state_dir = config.state_dir.clone();
    let fields_owned: Vec<String> = params.fields.to_vec();
    // Resolve --goal ref → goal resource id (trailing-UUID-only); the server filters on the live
    // `advances`→goal edge. An unparseable ref is a hard error (never a silent drop).
    let goal_id = params
        .goal
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(uuid::Uuid::from);
    let api_params = ResourceListParams {
        doc_type_name: Some(doc_type.clone()),
        context_ref: context.clone(),
        stage: params.stage.map(str::to_string),
        goal: goal_id,
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

    // Identity-out: every printed row carries its decorated `ref`.
    if let Some(rows) = envelope.get_mut("rows").and_then(|r| r.as_array_mut()) {
        for row in rows.iter_mut() {
            inject_ref(row);
        }
    }

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
    use temper_workflow::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let limit = params.limit.unwrap_or(50);
    let goal_id = params
        .goal
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(uuid::Uuid::from);
    let api_params = ResourceListParams {
        doc_type_name: Some(params.doc_type.to_string()),
        context_ref: params.context.map(ToString::to_string),
        stage: params.stage.map(str::to_string),
        goal: goal_id,
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        meta_only: Some(true),
        ..Default::default()
    };
    let fmt = params.format;
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

    let rendered = crate::format::render(&envelope, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Reassign a resource's owner via the API (`POST /api/resources/{id}/reassign`).
///
/// Auth is enforced server-side (current owner, or a team admin with reach over
/// the resource + target). The recipient is a bare profile UUID, matching the
/// `team` member commands.
/// `temper resource grant <ref> --to-profile|--to-team <ref> [--read] [--write] [--grant]`.
#[allow(clippy::too_many_arguments)]
pub fn grant(
    r#ref: &str,
    to_profile: Option<uuid::Uuid>,
    to_team: Option<String>,
    read: bool,
    write: bool,
    grant_cap: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resource_id = uuid::Uuid::from(temper_workflow::operations::parse_ref(r#ref)?);
    // A team ref is a decorated ref (UUID or `slug-<uuid>`); parse_ref keeps the trailing
    // UUID and ignores the slug half — no slug-uniqueness lookup needed.
    let to_team_id = to_team
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(uuid::Uuid::from);
    let principal = crate::actions::cogmap::resolve_principal(to_profile, to_team_id)?;

    let body = temper_core::types::resource_grant::ResourceGrantBody {
        principal_table: principal.table,
        principal_id: principal.id,
        can_read: read || write || grant_cap,
        can_write: write,
        can_delete: false,
        can_grant: grant_cap,
    };

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .grant(resource_id, &body)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `temper resource revoke <ref> --from-profile|--from-team <ref>`.
pub fn revoke(
    r#ref: &str,
    from_profile: Option<uuid::Uuid>,
    from_team: Option<String>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resource_id = uuid::Uuid::from(temper_workflow::operations::parse_ref(r#ref)?);
    let from_team_id = from_team
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(uuid::Uuid::from);
    let principal = crate::actions::cogmap::resolve_principal(from_profile, from_team_id)?;

    let body = temper_core::types::resource_grant::ResourceRevokeBody {
        principal_table: principal.table,
        principal_id: principal.id,
    };

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .revoke(resource_id, &body)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
}

pub fn reassign(r#ref: &str, to: &str, fmt: crate::format::OutputFormat) -> Result<()> {
    let id = temper_workflow::operations::parse_ref(r#ref)?;
    let to_profile_id = uuid::Uuid::parse_str(to.trim())
        .map_err(|e| TemperError::Api(format!("invalid profile id '{to}': {e}")))?;
    let req = temper_core::types::reassign::ReassignResourceRequest { to_profile_id };
    let ack = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .reassign(uuid::Uuid::from(id), &req)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;
    let rendered = crate::format::render(&ack, fmt)?;
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
    r#ref: &str,
    force: bool,
    act: temper_core::types::ActInput,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    use temper_workflow::operations::DeleteResource;

    let id = temper_workflow::operations::parse_ref(r#ref)?;

    // Context-free read: fetch the row by id to learn its context (for the
    // write backend), doctype + slug (for projection removal + result shape).
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let cmd = DeleteResource {
        resource: id,
        force,
        act: act.into_act_context()?,
        origin: temper_workflow::operations::Surface::CliCloud,
    };

    let (runtime, backend, _client) = crate::backend_select::build_backend(
        config,
        row.context_name.as_deref().unwrap_or_default(),
    )?;
    let output = runtime.block_on(backend.delete_resource(cmd))?;

    // Projection refresh: remove the resource's projection file. Best-effort
    // — a removal failure must not fail the (already-committed) delete.
    if let Err(e) =
        crate::projection::remove_resource_file_for_row(&config.vault_root, config, &row)
    {
        output::warning(format!("could not remove projection file: {e}"));
    }

    // `delete_resource` returns `CommandOutput<()>` — no row in scope.
    // Emit slug + doc_type from the fetched row (Task 9 flat result shape).
    let _ = output;
    let result = DeleteActionResult {
        status: "ok",
        slug: crate::actions::ingest::slug_from_title(&row.title),
        doc_type: row.doc_type_name.clone(),
    };
    let rendered = crate::format::render(&result, fmt)?;
    println!("{rendered}");

    Ok(())
}

/// Show a resource's content.
///
/// Cloud-only and context-free: the ref resolves to a `ResourceId`, the row +
/// content are fetched by id (no `resolve_by_uri`, no doctype dispatch — the
/// three former per-doctype shows rendered identically), the canonical
/// projection file is refreshed best-effort, and the row+body is rendered.
pub fn show(config: &Config, params: ShowParams<'_>) -> Result<()> {
    let id = temper_workflow::operations::parse_ref(params.r#ref)?;

    if params.meta_only {
        return show_meta_only(config, id, params.format, params.fields);
    }

    let config_clone = config.clone();
    let (mut metadata, body) = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let row = client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh — best-effort.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &row,
                &resp,
            ) {
                crate::output::warning(format!("could not refresh projection file: {e}"));
            }

            let metadata = serde_json::to_value(&row)
                .map_err(|e| TemperError::Api(format!("metadata serialize: {e}")))?;
            Ok((metadata, resp.markdown))
        })
    })?;

    inject_ref(&mut metadata);
    let rendered = crate::format::render_resource_show(&metadata, &body, params.format)?;
    println!("{rendered}");

    if params.edges {
        show_edges(config, id, params.format)?;
    }

    if params.provenance {
        show_provenance(config, id, params.format)?;
    }

    Ok(())
}

/// `show --meta-only`: hit GET /api/resources/{id}/meta and emit the
/// ResourceMetaResponse shape under the chosen format. Applies the
/// shared top-level projection filter when `fields` is non-empty.
///
/// Cloud-only and context-free: the id was already resolved from the ref by
/// `show`; this calls `get_meta` by id directly (no `resolve_by_uri`).
fn show_meta_only(
    _config: &Config,
    id: temper_core::types::ids::ResourceId,
    fmt: crate::format::OutputFormat,
    fields: &[String],
) -> Result<()> {
    use crate::actions::runtime;

    let fields_inner = fields.to_vec();

    let meta = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get_meta(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut value = serde_json::to_value(&meta)
        .map_err(|e| TemperError::Api(format!("meta serialize: {e}")))?;
    // Inject `ref` before the `--fields` filter (parity with `list`): the
    // anchor `resource_id` is always preserved, and `ref` is kept only when
    // requested — so `--fields` controls its visibility consistently.
    inject_ref(&mut value);
    let filtered =
        temper_core::projection::apply_top_level_filter(value, &fields_inner, "resource_id")
            .map_err(map_projection_error)?;
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

/// Fetch and display edges for a resource via the API.
///
/// Cloud-only and context-free: the id was already resolved from the ref by
/// `show`; this fetches and renders the edge list by id directly.
fn show_edges(
    _config: &Config,
    id: temper_core::types::ids::ResourceId,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    use crate::actions::runtime;

    let edges: Vec<temper_workflow::types::graph::GraphEdgeRow> = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .edges(uuid::Uuid::from(id))
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
    let rendered = crate::format::render(&report, fmt)?;
    println!("{rendered}");

    Ok(())
}

/// Fetch and display the itemized per-block provenance for a resource via the API.
///
/// Cloud-only and context-free: the id was already resolved from the ref by `show`; this
/// hits `GET /api/resources/{id}/provenance` and renders the rows in `(block, accretion)`
/// order. An unreadable resource returns an empty list (access-scoped in SQL).
fn show_provenance(
    _config: &Config,
    id: temper_core::types::ids::ResourceId,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    use crate::actions::runtime;

    let rows: Vec<temper_core::types::provenance::BlockProvenanceRow> =
        runtime::with_client(|client| {
            Box::pin(async move {
                client
                    .resources()
                    .provenance(uuid::Uuid::from(id))
                    .await
                    .map_err(crate::actions::runtime::client_err_to_temper)
            })
        })?;

    let rendered = crate::format::render(&rows, fmt)?;
    println!("{rendered}");

    Ok(())
}

/// Parameters for resource update.
pub struct UpdateParams<'a> {
    pub r#ref: &'a str,
    pub type_to: Option<&'a str>,
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
    /// Raw `--open-meta` JSON object string: arbitrary open-tier ("bring-your-own")
    /// keys, merged over the repeatable list flags above by `build_open_meta_for_update`.
    pub open_meta: Option<&'a str>,
    // Task-specific fields
    pub stage: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub seq: Option<i64>,
    pub branch: Option<&'a str>,
    pub pr: Option<&'a str>,
    /// Goal-set ref (`--goal`): resolved via `parse_ref`, folds any existing
    /// `advances`→goal edge and asserts the new one. Mutually exclusive with `clear_goal`.
    pub goal: Option<&'a str>,
    /// Goal-clear (`--clear-goal`): retract the resource's `advances`→goal edge.
    pub clear_goal: bool,
    // Goal-specific fields
    pub status: Option<&'a str>,
    /// Body source flag: `None` (rely on stdin auto-detection — non-empty piped
    /// stdin updates the body; empty implicit stdin means no body update),
    /// `Some("-")` (explicit stdin; errors if empty), or `Some("@<path>")`
    /// (read from file; errors if empty).
    pub body: Option<String>,
    /// Provenance source refs (`--sources`) — resolved to `ProvenanceSource::Resource`
    /// (refs) or `ProvenanceSource::Remote` (URLs) and attached to the addressed block.
    /// Requires a body update.
    pub sources: &'a [String],
    /// Which content block the body revise + `sources` target (`--content-block`, a block UUID).
    /// `None` → the resource's sole body block; `Some(id)` addresses that block explicitly.
    /// Requires a body update.
    pub content_block: Option<uuid::Uuid>,
    /// Output format, resolved globally upstream in `main`.
    pub format: crate::format::OutputFormat,
    /// Per-act correlation + authorship for the update act (from `--invocation`/`--confidence`/…).
    pub act: temper_core::types::ActInput,
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
) -> Option<temper_workflow::types::ManagedMeta> {
    // Identity (`--title`) travels first-class on the cmd, not through managed_meta —
    // this builder carries only the Property vocabulary.
    let any_set = params.stage.is_some()
        || params.mode.is_some()
        || params.effort.is_some()
        || params.seq.is_some()
        || params.branch.is_some()
        || params.pr.is_some()
        || params.status.is_some();
    if !any_set {
        return None;
    }
    Some(temper_workflow::types::ManagedMeta {
        stage: params.stage.map(String::from),
        mode: params.mode.map(String::from),
        effort: params.effort.map(String::from),
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

/// Parse a `--open-meta <json>` flag value into a validated open-tier object.
///
/// The open tier is a key/value map, so the value MUST be a JSON object; a
/// malformed string, or a JSON array/scalar, is a hard error rather than a
/// silent drop (parse-don't-validate / escalate). Returns the object `Value`.
fn parse_open_meta_flag(raw: &str) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| TemperError::BadRequest(format!("--open-meta must be valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(TemperError::BadRequest(
            "--open-meta must be a JSON object (e.g. '{\"marker\":\"x\"}')".into(),
        ));
    }
    Ok(value)
}

/// Combine the update surface's open-tier inputs into one `open_meta` object:
/// the repeatable list flags (`--tags`/`--relates-to`/…) form the base, then the
/// explicit `--open-meta` JSON object is merged over it (explicit keys win).
/// Returns `None` when neither source contributes a key (so a frontmatter-only
/// update with no open-tier change PATCHes nothing on the open tier).
fn build_open_meta_for_update(params: &UpdateParams<'_>) -> Result<Option<serde_json::Value>> {
    let mut obj = serde_json::Map::new();
    if let Some(serde_json::Value::Object(m)) = build_partial_open_meta_from_args(params) {
        obj.extend(m);
    }
    if let Some(raw) = params.open_meta {
        if let serde_json::Value::Object(m) = parse_open_meta_flag(raw)? {
            obj.extend(m);
        }
    }
    if obj.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::Value::Object(obj)))
    }
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

/// Build a `MoveSpec` from the `--type-to` CLI flag. Returns `None` when
/// `type_to` is not set.
///
/// Context moves (`--context-to`) do NOT produce a `MoveSpec` here: the CLI
/// can't resolve a context ref to a `ContextId` without DB access. Instead,
/// the raw ref string travels via `UpdateResource.context_ref` and is
/// forwarded verbatim by the cloud-backend translator as `context_to` in the
/// HTTP wire payload, where the API handler resolves it server-side.
fn build_move_spec_from_args(
    params: &UpdateParams<'_>,
) -> Option<temper_workflow::operations::MoveSpec> {
    params
        .type_to
        .map(|tt| temper_workflow::operations::MoveSpec {
            context_to: None,
            type_to: Some(String::from(tt)),
        })
}

/// Resolve the update target: parse the ref to an id, read the current
/// server row (context-free) for its doctype + home context, and validate
/// the current doctype and any `--type-to` target before the command is
/// built. Returns the `(id, row)` pair the rest of `update` threads on.
fn resolve_update_target(
    params: &UpdateParams<'_>,
) -> Result<(
    temper_core::types::ids::ResourceId,
    temper_workflow::types::resource::ResourceRow,
)> {
    let id = temper_workflow::operations::parse_ref(params.r#ref)?;
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;
    let _ = temper_workflow::frontmatter::DocType::from_str(&row.doc_type_name)?;
    if let Some(tt) = params.type_to {
        let _ = temper_workflow::frontmatter::DocType::from_str(tt)?;
    }
    Ok((id, row))
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

    use temper_workflow::operations::{BodyUpdate, GoalPatch, UpdateResource};

    // 1. Resolve the ref to an id + the current server row (for its doctype
    //    and home context), validating the doctype and any `--type-to`.
    let (id, row) = resolve_update_target(params)?;
    let current_type = row.doc_type_name.clone();

    // 2. Per-flag schema validation, keyed by the resolved doctype.
    validate_update_args(params, &current_type)?;

    // 3. --body resolution.
    let stdin_is_tty = std::io::stdin().is_terminal();
    let resolved_body = crate::actions::body_source::resolve_body_source(
        params.body.as_deref(),
        stdin_is_tty,
        std::io::stdin(),
        crate::actions::body_source::stdin_has_input_within,
    )?;

    // 3b. Resolve --sources refs → provenance records. A ref that fails to parse is a hard
    // error (escalate); sources without a body update have nothing to attribute.
    let resolved_sources = resolve_provenance_sources(params.sources)?;
    if !resolved_sources.is_empty() && resolved_body.is_none() {
        return Err(TemperError::BadRequest(
            "--sources requires a body update; add --body or pipe content".into(),
        ));
    }
    // --content-block addresses which block the body revise targets; with no body there is
    // nothing to write to it.
    if params.content_block.is_some() && resolved_body.is_none() {
        return Err(TemperError::BadRequest(
            "--content-block requires a body update; add --body or pipe content".into(),
        ));
    }

    // 3c. Goal patch: --goal (set/replace, ref resolved via parse_ref) wins; --clear-goal
    // retracts; neither leaves the goal edge untouched. clap's `conflicts_with` guarantees at
    // most one is set, so the ordering here is defensive, not load-bearing.
    let goal = match (params.goal, params.clear_goal) {
        (Some(r), _) => Some(GoalPatch::Set(temper_workflow::operations::parse_ref(r)?)),
        (None, true) => Some(GoalPatch::Clear),
        (None, false) => None,
    };

    // 4. Build the UpdateResource cmd.
    // context_to travels as a raw ref via context_ref (the API handler resolves
    // it server-side); type_to goes through MoveSpec and travels first-class on
    // the wire (type is no longer a managed_meta key).
    let cmd = UpdateResource {
        resource: id,
        title: params.title.map(String::from),
        // CLI update has no --slug flag; the server derives the slug from an
        // effective title change.
        slug: None,
        body: resolved_body.map(|content| {
            let mut body = BodyUpdate::new(content);
            body.sources = resolved_sources;
            body.content_block = params.content_block;
            body
        }),
        managed_meta: build_partial_managed_meta_from_args(params),
        open_meta: build_open_meta_for_update(params)?,
        goal,
        move_to: build_move_spec_from_args(params),
        context_ref: params.context_to.map(String::from),
        act: params.act.clone().into_act_context()?,
        origin: temper_workflow::operations::Surface::CliCloud,
    };

    // 5. Acquire the cloud backend + client and dispatch the update.
    let (runtime, backend, client) = crate::backend_select::build_backend(
        config,
        row.context_name.as_deref().unwrap_or_default(),
    )?;
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
    let rendered = crate::format::render(&result, params.format)?;
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
    fn empty_update_params(r#ref: &str) -> UpdateParams<'_> {
        UpdateParams {
            r#ref,
            type_to: None,
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
            open_meta: None,
            stage: None,
            mode: None,
            effort: None,
            seq: None,
            branch: None,
            pr: None,
            goal: None,
            clear_goal: false,
            status: None,
            body: None,
            sources: &[],
            content_block: None,
            format: crate::format::OutputFormat::Json,
            act: temper_core::types::ActInput::default(),
        }
    }

    #[test]
    fn build_move_spec_returns_none_when_both_flags_unset() {
        let params = empty_update_params("foo");
        assert!(build_move_spec_from_args(&params).is_none());
    }

    /// context_to goes via `context_ref` in `UpdateResource`, not through
    /// `MoveSpec.context_to`: the CLI can't resolve a ref to a ContextId
    /// without DB access, so MoveSpec.context_to is always None from the CLI.
    #[test]
    fn build_move_spec_returns_none_when_only_context_to_set() {
        let mut params = empty_update_params("foo");
        params.context_to = Some("@me/temper");
        // MoveSpec is None when only context_to is provided; the ref is
        // forwarded via UpdateResource.context_ref by the caller instead.
        assert!(
            build_move_spec_from_args(&params).is_none(),
            "context_to alone must not produce a MoveSpec; raw ref goes via context_ref"
        );
    }

    #[test]
    fn build_move_spec_returns_some_with_type_to_when_set() {
        let mut params = empty_update_params("foo");
        params.type_to = Some("concept");
        let spec = build_move_spec_from_args(&params).expect("expected Some with type_to");
        assert_eq!(
            spec.context_to, None,
            "MoveSpec.context_to is always None from CLI"
        );
        assert_eq!(spec.type_to, Some("concept".to_string()));
    }

    #[test]
    fn build_move_spec_returns_some_with_type_to_when_both_set() {
        // context_to goes via context_ref; type_to is still in MoveSpec.
        let mut params = empty_update_params("foo");
        params.context_to = Some("@me/temper");
        params.type_to = Some("concept");
        let spec = build_move_spec_from_args(&params).expect("expected Some with type_to");
        assert_eq!(
            spec.context_to, None,
            "context_to never in MoveSpec from CLI"
        );
        assert_eq!(spec.type_to, Some("concept".to_string()));
    }

    // Identity (`--title`) is a first-class wire field since Phase 2 — it travels
    // on `UpdateResource.title`, not through `build_partial_managed_meta_from_args`
    // (which now carries only the Property vocabulary). The former "title propagates
    // through the partial managed_meta" guards were removed with that reshape.

    // --- issue #307: --open-meta arbitrary open-tier keys (create + update) ---

    #[test]
    fn parse_open_meta_flag_accepts_object() {
        let v = parse_open_meta_flag(r#"{"marker":"x","n":1}"#).expect("valid object");
        assert_eq!(v.get("marker"), Some(&serde_json::json!("x")));
        assert_eq!(v.get("n"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn parse_open_meta_flag_rejects_non_object_and_malformed() {
        // A JSON array/scalar is not a key/value map → hard error.
        assert!(parse_open_meta_flag(r#"["a","b"]"#).is_err());
        assert!(parse_open_meta_flag("42").is_err());
        // Malformed JSON → hard error (never a silent drop).
        assert!(parse_open_meta_flag("{not json").is_err());
    }

    #[test]
    fn build_open_meta_for_update_merges_explicit_over_list_flags() {
        let mut params = empty_update_params("foo");
        let tags = vec!["a".to_string(), "b".to_string()];
        params.tags = &tags;
        params.open_meta = Some(r#"{"marker":"x"}"#);
        let out = build_open_meta_for_update(&params)
            .expect("ok")
            .expect("some open_meta");
        assert_eq!(out.get("tags"), Some(&serde_json::json!(["a", "b"])));
        assert_eq!(out.get("marker"), Some(&serde_json::json!("x")));
    }

    #[test]
    fn build_open_meta_for_update_is_none_when_no_open_tier_input() {
        let params = empty_update_params("foo");
        assert!(build_open_meta_for_update(&params).expect("ok").is_none());
    }

    #[test]
    fn build_open_meta_for_update_propagates_malformed_flag_error() {
        let mut params = empty_update_params("foo");
        params.open_meta = Some("{bad");
        assert!(build_open_meta_for_update(&params).is_err());
    }
}

#[cfg(test)]
mod action_result_tests {
    use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
    use temper_workflow::types::resource::ResourceRow;

    use super::{CreateActionResult, DeleteActionResult, UpdateActionResult};

    /// Build a minimal `ResourceRow` fixture for action result tests.
    pub(super) fn make_resource_row(
        _slug: &str,
        doc_type: &str,
        title: &str,
        context: &str,
    ) -> ResourceRow {
        ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: Some(ContextId(uuid::Uuid::nil())),
            origin_uri: "test://origin".to_string(),
            title: title.to_string(),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: Some(context.to_string()),
            doc_type_name: doc_type.to_string(),
            owner_handle: "@me".to_string(),
            context_slug: Some(context.to_string()),
            context_owner_ref: Some("@me".to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
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
        assert!(out.contains("\"title\""), "title missing: {out}");
        assert!(
            out.contains("\"context_name\""),
            "context_name missing: {out}"
        );
        assert!(
            out.contains("\"doc_type_name\""),
            "doc_type_name missing: {out}"
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
        assert!(
            out.contains("\"doc_type_name\""),
            "doc_type_name missing: {out}"
        );
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
    use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
    use temper_workflow::types::resource::ResourceRow;

    /// Task 7: verify that `render()` passthrough includes internal wire fields
    /// like `body_hash` that the old `row_to_frontmatter_value` + `render_server_rows`
    /// path deliberately dropped. This is the canary for the breaking change.
    #[test]
    fn render_resource_list_json_passes_wire_type_with_internals() {
        let rows: Vec<ResourceRow> = vec![ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: Some(ContextId(uuid::Uuid::nil())),
            origin_uri: "test://origin".to_string(),
            title: "Test Resource".to_string(),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            context_name: Some("temper".to_string()),
            doc_type_name: "research".to_string(),
            owner_handle: "@me".to_string(),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: Some("abc123deadbeef".to_string()),
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
        assert!(
            out.contains("\"doc_type_name\""),
            "wire field 'doc_type_name' missing: {out}"
        );
    }
}

/// Tests for the `EdgesReport` struct and its render path.
#[cfg(test)]
mod edges_report_tests {
    use super::EdgesReport;
    use temper_core::types::graph::{EdgeKind, Polarity};
    use temper_core::types::ids::{EdgeId, ResourceId};
    use temper_workflow::types::graph::GraphEdgeRow;

    fn make_edge(direction: &str, label: &str) -> GraphEdgeRow {
        GraphEdgeRow {
            edge_id: EdgeId::from(uuid::Uuid::nil()),
            peer_resource_id: ResourceId::from(uuid::Uuid::nil()),
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
mod inject_ref_tests {
    #[test]
    fn inject_ref_adds_decorated_form_from_title_and_id() {
        let mut row = serde_json::json!({
            "id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "title": "My Task",
        });
        super::inject_ref(&mut row);
        assert_eq!(
            row.get("ref").and_then(|v| v.as_str()),
            Some("my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2")
        );
    }
}

#[cfg(test)]
mod show_meta_only_tests {
    use temper_core::projection::apply_top_level_filter;
    use temper_workflow::types::managed_meta::{ManagedMeta, ResourceMetaResponse};

    fn fake_meta_response() -> ResourceMetaResponse {
        ResourceMetaResponse {
            resource_id: temper_core::types::ResourceId::from(uuid::Uuid::nil()),
            managed_meta: Some(ManagedMeta {
                stage: Some("in-progress".to_string()),
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
