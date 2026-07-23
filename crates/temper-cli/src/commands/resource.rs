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
    /// Targets of the `derived_from` edges asserted by `--sources-as-edges`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edges_asserted: Vec<uuid::Uuid>,
    /// Sources whose edge assert failed. The resource exists; re-assert with
    /// `temper edge assert` (idempotent) rather than re-running the create.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edges_failed: Vec<uuid::Uuid>,
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
/// Reads the anchor id from `id` (ResourceRow / ResourceDetail /
/// ResourceMetaResponse) OR `resource_id` (UnifiedSearchResultRow, which still
/// anchors on the longer name). Both branches are live — do not collapse them.
/// No-op if the id is absent or unparseable.
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
    // A row carrying no `title` — the `--meta-only` projection is the one that doesn't —
    // cannot form the decorated half of a ref. This used to default the title to `""` and
    // emit `-<uuid>`: a malformed ref that resolved only by accident (resolution is
    // trailing-UUID-only) and that made the meta projection disagree with the full `show`
    // on the value of `ref`. Emit nothing instead; a bare UUID is itself a valid ref.
    let Some(title) = row.get("title").and_then(|v| v.as_str()) else {
        return;
    };
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

/// Render a create/update action result with its decorated `ref` injected, the way
/// `list`/`show`/`search` rows carry one.
///
/// `create` and `update` used to serialize their typed result struct directly, so they were
/// the only resource-returning commands whose output had no `ref` — an agent that had just
/// made a resource needed a second round-trip to address it. The result is serialized to a
/// `Value` first (as `list` does), `inject_ref` decorates it, and the whole thing renders as
/// exactly one document.
fn render_action_result_with_ref<T: serde::Serialize>(
    result: &T,
    fmt: crate::format::OutputFormat,
) -> Result<String> {
    let mut value = serde_json::to_value(result)
        .map_err(|e| TemperError::Api(format!("action result serialize: {e}")))?;
    inject_ref(&mut value);
    crate::format::render(&value, fmt)
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
/// when `from` conflicts with `--body` or with a genuinely piped stdin body,
/// when the path does not exist, or when extraction fails.
///
/// **Stdin gate (issue #420 item 1).** A non-TTY stdin is *not* on its own a
/// conflict — every agent harness, CI job, and `< /dev/null` invocation has one,
/// which is exactly where `--from` is most useful. The gate fires only when stdin
/// actually carries a body: the readiness probe short-circuits an open-but-idle
/// pipe (poll times out → no read), and an at-EOF stdin (`< /dev/null`) reads to
/// empty → no conflict. Only real piped bytes (`cat foo | temper … --from bar`)
/// error. `--from` wins regardless, so the drained stdin is discarded either way.
///
/// URL detection: strings with `http://` or `https://` prefix are fetched to a
/// tempfile first, then extracted. A `file://` URI is decoded to a local path
/// (`resolve_from_local_path`) and read like any other local file. Everything
/// else is treated as a plain local path.
async fn resolve_from_input<R: std::io::Read>(
    from: Option<&str>,
    body_flag: Option<&str>,
    stdin_is_tty: bool,
    mut stdin_reader: R,
    stdin_ready: impl FnOnce() -> bool,
) -> Result<Option<String>> {
    let Some(from) = from else { return Ok(None) };

    if body_flag.is_some() {
        return Err(TemperError::Config(
            "--from cannot be combined with --body".to_string(),
        ));
    }
    // Only a non-TTY stdin that actually has bytes ready is a real `--body`-vs-`--from`
    // collision. Probe first (idle-open pipe → not ready → never read → no hang), then read
    // (EOF/`< /dev/null` → empty → not a conflict). See the doc comment above.
    if !stdin_is_tty && stdin_ready() {
        let mut buf = String::new();
        stdin_reader
            .read_to_string(&mut buf)
            .map_err(|e| TemperError::Vault(format!("read stdin: {e}")))?;
        if !buf.is_empty() {
            return Err(TemperError::Config(
                "--from cannot be combined with a piped stdin body; pass one or the other"
                    .to_string(),
            ));
        }
    }

    let extracted = if temper_workflow::operations::is_remote_url(from) {
        let (tmp, _name) = crate::actions::ingest::fetch_url_to_tempfile(from).await?;
        crate::extract::extract_to_markdown(tmp.as_ref()).await?
    } else {
        let path = resolve_from_local_path(from)?;
        if !path.exists() {
            return Err(TemperError::Config(format!(
                "--from path does not exist: {}",
                path.display()
            )));
        }
        crate::extract::extract_to_markdown(&path).await?
    };

    // An extractor that finds no text does not error — it returns Ok(""). A scanned or image-only
    // PDF is the common case: structurally valid, opens fine, has no text layer to give.
    //
    // Left alone, that empty string is filtered to None downstream and the backend synthesizes
    // `# {title}` in its place, so the command would exit 0, print a ref, and store a title-only
    // resource with the document silently gone. That is the same class of bug as #420 item 3
    // (a silently-partial ingest), and it is worse than the failure it replaced: before PDF
    // support, this input failed loudly and told you to convert the file.
    //
    // Refuse, the way an explicit empty `--body` already does.
    if extracted.content.trim().is_empty() {
        let remedy = if extracted.mime_type == "application/pdf" {
            "it has no text layer — a scanned or image-only PDF. Run it through OCR first \
             (e.g. `ocrmypdf in.pdf out.pdf`), or pass the text with --body"
        } else {
            "it yielded no text"
        };
        return Err(TemperError::Config(format!(
            "--from extracted no text from '{from}': {remedy}"
        )));
    }

    Ok(Some(extracted.content))
}

/// Resolve the local-file half of `--from` to a filesystem path.
///
/// A plain path is taken verbatim. A `file://` URI is decoded to a local path via the `url` crate —
/// handling percent-escapes (`%20` → space) and the empty/`localhost` authority — so
/// `--from file:///a/b%20c.pdf` "just works" the way passing the plain path does. This is deliberate
/// forgiveness: `file://` is a spelling agents naturally reach for (it is what `--sources` accepts),
/// and a decoded local path is exactly a plain path, so the two converge on one existence-check +
/// extract. A `file://` URI with a non-local authority (`file://otherhost/…`) has no local path and
/// is a hard error rather than a silent wrong target (parse-don't-validate / escalate).
fn resolve_from_local_path(from: &str) -> Result<std::path::PathBuf> {
    if from.starts_with("file://") {
        let url = url::Url::parse(from).map_err(|e| {
            TemperError::Config(format!("--from: invalid file:// URI '{from}': {e}"))
        })?;
        url.to_file_path().map_err(|()| {
            TemperError::Config(format!(
                "--from: '{from}' is not a local file:// path (a remote authority cannot be read); \
                 pass a plain filesystem path"
            ))
        })
    } else {
        Ok(std::path::PathBuf::from(from))
    }
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
    /// `--sources-as-edges` — also assert a `derived_from` edge to each resource-valued
    /// source, in addition to the block-provenance record. Gated on `sources` by clap
    /// (`requires = "sources"`).
    pub sources_as_edges: bool,
    /// `--no-source` — suppress the `--from <url>` provenance default (issue #352). When a URL
    /// `--from` is given without explicit `--sources`, the resource's `origin_uri` is set to that
    /// URL and the server seeds a Remote block-provenance record from it; this opt-out preserves
    /// the pre-#352 behavior (empty `origin_uri`, no provenance). Clap-exclusive with `sources`.
    pub no_source: bool,
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

/// The subset of `--sources` that can become `derived_from` graph edges.
///
/// Only `ProvenanceSource::Resource` has a resource target. `Remote` (an external URL)
/// and `Event` (a kb_events id) are recorded as block provenance but have no node to
/// point an edge at, so they are silently skipped rather than erroring — citing a URL
/// alongside two resources is a normal thing to do.
fn source_edge_targets(
    sources: &[temper_core::types::provenance::ProvenanceSource],
) -> Vec<uuid::Uuid> {
    use temper_core::types::provenance::ProvenanceSource;
    sources
        .iter()
        .filter_map(|s| match s {
            ProvenanceSource::Resource(id) => Some(*id),
            ProvenanceSource::Remote(_) | ProvenanceSource::Event(_) => None,
        })
        .collect()
}

/// Derive the created resource's `origin_uri` from `--from` (issue #352). A remote (http/https)
/// `--from` URL becomes the resource's origin — server-side this seeds a Remote block-provenance
/// record when no explicit `--sources` are given, making `create --from <url>` citation-grade by
/// default. A local `--from` path has no external origin (returns `None`), and `--no-source` opts
/// out entirely (preserving the pre-#352 empty-`origin_uri`, no-provenance behavior).
fn origin_uri_from_source(from: Option<&str>, no_source: bool) -> Option<String> {
    if no_source {
        return None;
    }
    from.filter(|f| temper_workflow::operations::is_remote_url(f))
        .map(str::to_owned)
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
        open_meta,
        goal,
        task,
        body_flag,
        from,
        sources,
        sources_as_edges,
        no_source,
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

    // Slug is §7-dissolved (never stored; addressing is trailing-UUID-only), so it is NOT a
    // caller input — always derived from the title. It seeds the client-side `validate_create`
    // temper-slug check; the server re-derives its own from the title (issue #307 Bug 2). The
    // date-prefix for non-Concept/Goal doctypes is retained for the local projection filename.
    let doctype_enum = temper_workflow::frontmatter::DocType::from_str(doc_type).ok();
    let slug_resolved = derive_create_slug(title, doctype_enum);

    // Parse the optional --open-meta JSON object (the free-form open tier) and validate its shape
    // send-side (the server re-enforces the same gate — symmetric defense).
    let open_meta_value = open_meta.map(parse_open_meta_flag).transpose()?;
    if let Some(om) = &open_meta_value {
        validate_open_meta_send_side(om)?;
    }

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

    // `resolved_sources` is moved into `cmd.body` below; `--sources-as-edges` needs its
    // own copy to select edge targets after the create (build_backend/create_resource
    // consume `cmd`, so we can't reach back into it post-create).
    let sources_for_edges = resolved_sources.clone();

    // Resolve --goal ref → goal resource id (trailing-UUID-only, like `edge assert`); the server
    // projects the live `advances`→goal edge after create. An unparseable ref is a hard error.
    let goal_resolved = goal
        .map(temper_workflow::operations::parse_ref)
        .transpose()?;

    // `act` (an `ActInput`) is consumed by `.into_act_context()?` below; `--sources-as-edges`
    // needs its own copy to attach authorship to the post-create edge asserts.
    let act_for_edges = act.clone();

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
        // A URL `--from` becomes the resource's origin (issue #352); the server seeds a Remote
        // block-provenance record from it when no explicit `--sources` were given. `--no-source`
        // and a local-path `--from` leave this `None`.
        origin_uri: origin_uri_from_source(from.as_deref(), no_source),
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

    // Acquire the cloud backend + client and dispatch the create. A body over
    // `SEGMENT_BUDGET_BYTES` streams through the segmented ingest endpoints (Beat 3,
    // `actions::ingest::run_segmented_create`); everything at or under the budget takes the
    // existing one-shot `create_resource` path, unchanged (`actions::ingest::ingest_mode` is
    // the seam that decides). Segmented dispatch is embed-gated exactly like the one-shot
    // path's own body-trio computation (`compute_body_chunks`) already is — a no-embed build
    // falls straight through to `backend.create_resource`, which already returns the
    // "cloud mode requires --features embed" error for any body, so no separate fallback
    // message is needed there.
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &ctx)?;

    #[cfg(feature = "embed")]
    let created_resource = {
        let body_len = cmd.body.as_ref().map(|b| b.content.len()).unwrap_or(0);
        let budget = temper_ingest::stream::SEGMENT_BUDGET_BYTES;
        if crate::actions::ingest::ingest_mode(body_len, budget)
            == crate::actions::ingest::IngestMode::Segmented
        {
            let params = crate::actions::ingest::SegmentedCreateParams {
                client: &client,
                vault_root: &config.vault_root,
                cmd: &cmd,
                context_ref: &ctx,
                budget,
            };
            runtime.block_on(crate::actions::ingest::run_segmented_create(params))?
        } else {
            runtime.block_on(backend.create_resource(cmd))?.value
        }
    };
    #[cfg(not(feature = "embed"))]
    let created_resource = runtime.block_on(backend.create_resource(cmd))?.value;

    // Projection refresh: write the new resource to its canonical
    // projection path so the local copy reflects server state at once.
    // Best-effort — a projection write failure must not fail the create.
    if let Err(e) = runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &created_resource,
    )) {
        output::warning(format!("could not write projection file: {e}"));
    }

    // Session→task linking. Only reached for sessions (validated fail-fast
    // above). The session resource is already created; the link is a best-
    // effort tail — an unknown task warns and skips rather than failing the
    // (already-committed) create.
    if let Some(task_slug) = task {
        link_session_to_task(
            config,
            &runtime,
            &client,
            &ctx,
            created_resource.id,
            task_slug,
        );
    }

    // `--sources-as-edges`: one `derived_from` edge per resource-valued source.
    //
    // Deliberately NOT atomic and deliberately NOT fatal. The create has already
    // committed and is not idempotent (content dedup was retired, #219), so failing
    // here would push an author toward re-running the create and duplicating the
    // node. `relationship_assert` upserts on the active-edge invariant, so a failed
    // edge is safely re-assertable with `temper edge assert`. Mirrors `link_session_to_task`.
    let (edges_asserted, edges_failed) = if sources_as_edges {
        use temper_core::types::relationship_requests::AssertRelationshipRequest;
        // Structural triple for the frontmatter `derived_from` relation — sourced from
        // the one legacy-mapping table so the CLI never restates it by hand.
        let (edge_kind, polarity, label) =
            temper_workflow::types::graph::EdgeType::DerivedFrom.legacy_mapping();

        let targets = source_edge_targets(&sources_for_edges);
        let mut asserted = Vec::new();
        let mut failed = Vec::new();

        for target in targets {
            let req = AssertRelationshipRequest {
                source: created_resource.id,
                target: temper_core::types::ids::ResourceId::from(target),
                edge_kind,
                polarity,
                label: label.to_string(),
                weight: 1.0,
                act: act_for_edges.clone(),
            };
            let outcome = runtime.block_on(client.relationships().assert(&req));
            match outcome {
                Ok(_) => asserted.push(target),
                Err(e) => {
                    output::warning(format!(
                        "could not assert derived_from edge to {target}: {e} \
                         (resource created; re-run `temper edge assert` — it is idempotent)"
                    ));
                    failed.push(target);
                }
            }
        }
        (asserted, failed)
    } else {
        (Vec::new(), Vec::new())
    };

    let result = CreateActionResult {
        status: "ok",
        resource: created_resource,
        edges_asserted,
        edges_failed,
    };
    let rendered = render_action_result_with_ref(&result, format)?;
    crate::output::plain(rendered);
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
        rt.block_on(resolve_from_input(
            from,
            body_flag,
            stdin_is_tty,
            std::io::stdin(),
            crate::actions::body_source::stdin_has_input_within,
        ))?
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

/// Derive a resource slug from the title (slug is §7-dissolved and never a caller
/// input — issue #307). Date-prefixes every doctype except Concept and Goal (which
/// are identified by name). `doctype` is `None` for an unrecognized (open-tail)
/// label, which falls into the date-prefixed catch-all alongside every other
/// non-Concept/Goal doctype. Used for the client-side `validate_create` temper-slug
/// check and the local projection filename; the server re-derives its own.
fn derive_create_slug(
    title: &str,
    doctype: Option<temper_workflow::frontmatter::DocType>,
) -> String {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let base_slug = vault::slugify(title);
    match doctype {
        // Concept and Goal are identified by name — no date prefix.
        Some(temper_workflow::frontmatter::DocType::Concept)
        | Some(temper_workflow::frontmatter::DocType::Goal) => base_slug,
        // Every other doctype (known or open-tail/unrecognized) gets a date prefix.
        _ => format!("{today}-{base_slug}"),
    }
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
                    .map_err(crate::actions::runtime::client_err_to_temper)
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
    pub lineage: bool,
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
    /// `--all`: return every matching row (no page cap). Overrides `limit`
    /// (clap makes the two mutually exclusive, so both are never set together).
    pub all: bool,
    /// `--offset`: skip the first N matching rows (pagination).
    pub offset: Option<usize>,
    /// `--sort <field>[:asc|desc]`. Parsed by `parse_sort_arg`; `None` keeps
    /// the default `updated:desc`.
    pub sort: Option<&'a str>,
    /// `--title-contains`: case-insensitive title substring filter (the list
    /// `q`, a trivial `ILIKE` — full text/vector search is `temper search`).
    pub title_contains: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub status: Option<&'a str>,
    pub format: crate::format::OutputFormat,
    pub meta_only: bool,
    pub fields: &'a [String],
}

/// The default page cap for `list` when neither `--limit` nor `--all` is given.
/// `--meta-only` uses [`DEFAULT_META_LIST_LIMIT`]. Kept small enough to be cheap,
/// large enough that the common case fits — but the `total`/`truncated` signal
/// makes any cap self-evident, so an agent never has to guess whether it saw
/// the whole set.
const DEFAULT_LIST_LIMIT: usize = 20;
/// The default page cap for `list --meta-only` (meta rows are cheaper, so the
/// default is larger).
const DEFAULT_META_LIST_LIMIT: usize = 50;

/// Parse a `--sort <field>[:asc|desc]` argument into an enum pair. The field
/// half is matched against a small alias set; the direction half is optional
/// and defaults per field (time/seq → desc, textual → asc) so a bare
/// `--sort title` reads alphabetically without the caller spelling out `:asc`.
///
/// A bad field or direction is a hard error (escalate, never silently ignore) —
/// silently mis-sorting a list is exactly the class of footgun this task fixes.
fn parse_sort_arg(
    raw: &str,
) -> Result<(
    temper_workflow::types::resource::ResourceSortField,
    temper_workflow::types::resource::SortOrder,
)> {
    use temper_workflow::types::resource::{ResourceSortField, SortOrder};

    let (field_str, dir_str) = match raw.split_once(':') {
        Some((f, d)) => (f.trim(), Some(d.trim())),
        None => (raw.trim(), None),
    };

    let field = match field_str.to_ascii_lowercase().as_str() {
        "updated" | "updated-at" | "updated_at" => ResourceSortField::Updated,
        "created" | "created-at" | "created_at" => ResourceSortField::Created,
        "title" => ResourceSortField::Title,
        "stage" => ResourceSortField::Stage,
        "seq" => ResourceSortField::Seq,
        "context" | "context-name" | "context_name" => ResourceSortField::ContextName,
        "doctype" | "doc-type" | "doc_type" | "type" => ResourceSortField::DocTypeName,
        other => {
            return Err(TemperError::BadRequest(format!(
                "--sort: unknown field '{other}' \
                 (expected one of: updated, created, title, stage, seq, context, doctype)"
            )));
        }
    };

    let order = match dir_str {
        None => match field {
            // Time and sequence sort newest/highest-first by default.
            ResourceSortField::Updated | ResourceSortField::Created | ResourceSortField::Seq => {
                SortOrder::Desc
            }
            // Textual fields read most naturally in ascending (A→Z) order.
            ResourceSortField::Title
            | ResourceSortField::Stage
            | ResourceSortField::ContextName
            | ResourceSortField::DocTypeName => SortOrder::Asc,
        },
        Some(d) => match d.to_ascii_lowercase().as_str() {
            "asc" | "ascending" => SortOrder::Asc,
            "desc" | "descending" => SortOrder::Desc,
            other => {
                return Err(TemperError::BadRequest(format!(
                    "--sort: unknown direction '{other}' (expected 'asc' or 'desc')"
                )));
            }
        },
    };

    Ok((field, order))
}

/// Resolve the effective page limit for a list call. `--all` means "no cap"
/// (`None` — the server returns every matching row); otherwise the explicit
/// `--limit`, falling back to `default`.
fn resolve_list_limit(all: bool, limit: Option<usize>, default: usize) -> Option<i64> {
    if all {
        None
    } else {
        Some(limit.unwrap_or(default) as i64)
    }
}

/// Inject the truncation signal into a `list`/`list --meta-only` envelope and
/// report whether the page was capped.
///
/// The server already returns `total` (the FILTERED match count, before
/// limit/offset) alongside `rows`. Silent truncation — reasoning over a capped
/// page as if it were the whole set — is the root footgun this task fixes, so
/// we surface it two ways: a machine-readable `truncated` boolean on the
/// envelope, and (via the returned bool) a stderr hint for humans. `truncated`
/// is true iff there are matching rows beyond this page (`offset + returned <
/// total`). Also injects `returned` (this page's row count) for symmetry with
/// `total`.
fn inject_truncation_signal(envelope: &mut serde_json::Value, offset: usize) -> bool {
    let obj = match envelope.as_object_mut() {
        Some(o) => o,
        None => return false,
    };
    let returned = obj
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let total = obj.get("total").and_then(|t| t.as_i64()).unwrap_or(0);
    let truncated = (offset as i64) + (returned as i64) < total;
    obj.insert("returned".to_string(), serde_json::json!(returned));
    obj.insert("truncated".to_string(), serde_json::json!(truncated));
    truncated
}

/// Emit the stderr note shown when a `list` page is truncated. Routed through
/// `output::warning` rather than `output::hint` for *severity*, not for stream
/// choice — both now write to stderr, so neither can corrupt the JSON document
/// an agent parses on stdout. A capped page an agent silently mistakes for the
/// whole set is a wrong answer, not a suggestion. Names the exact escape
/// hatches (`--all`, a bigger `--limit`, `--offset`, or narrowing with
/// `--sort`/filters) so an agent self-corrects instead of asserting a set is
/// complete from a capped page.
fn warn_truncated(total: i64, returned: usize) {
    output::warning(format!(
        "Showing {returned} of {total} matching results — the list is TRUNCATED. \
         Do not conclude a resource is absent or a set is complete from this page. \
         Re-run with --all (or a larger --limit/--offset), or narrow with \
         --title-contains/--stage/--sort first."
    ));
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
    use temper_workflow::types::resource::ResourceListParams;

    let fmt = params.format;
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = resolve_list_limit(params.all, params.limit, DEFAULT_LIST_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let (sort, order) = match params.sort {
        Some(raw) => {
            let (f, o) = parse_sort_arg(raw)?;
            (Some(f), Some(o))
        }
        None => (None, None),
    };
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
        q: params.title_contains.map(str::to_string),
        goal: goal_id,
        sort,
        order,
        limit,
        offset: Some(offset as i64),
        ..Default::default()
    };

    // Cloud-only list: the server query. Any error (network, auth, 4xx/5xx)
    // surfaces as-is — there is no local-scan fallback.
    let response = runtime::with_client(move |client| {
        Box::pin(async move {
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

    // Truncation signal — injected BEFORE the optional `--fields` projection so
    // the `total`/`returned`/`truncated` envelope keys are always present (they
    // survive the filter, which only prunes per-row keys, not envelope keys).
    let total = envelope.get("total").and_then(|t| t.as_i64()).unwrap_or(0);
    let returned = envelope
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let truncated = inject_truncation_signal(&mut envelope, offset);

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
    if truncated {
        warn_truncated(total, returned);
    }
    Ok(())
}

/// `list --meta-only`: call client.resources().list_meta() and emit the
/// meta-list envelope, whose rows are now full `ResourceDetail`s (row + both
/// meta tiers per item — the whole view minus each body). Injects each row's
/// decorated `ref` (rows carry a title now), then applies the shared top-level
/// projection filter to each row when `fields` is non-empty; the envelope keys
/// (`rows`, `total`, `facets`) are preserved untouched.
fn list_meta_only(_config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_workflow::types::resource::ResourceListParams;

    let limit = resolve_list_limit(params.all, params.limit, DEFAULT_META_LIST_LIMIT);
    let offset = params.offset.unwrap_or(0);
    let (sort, order) = match params.sort {
        Some(raw) => {
            let (f, o) = parse_sort_arg(raw)?;
            (Some(f), Some(o))
        }
        None => (None, None),
    };
    let goal_id = params
        .goal
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(uuid::Uuid::from);
    let api_params = ResourceListParams {
        doc_type_name: Some(params.doc_type.to_string()),
        context_ref: params.context.map(ToString::to_string),
        stage: params.stage.map(str::to_string),
        q: params.title_contains.map(str::to_string),
        goal: goal_id,
        sort,
        order,
        limit,
        offset: Some(offset as i64),
        meta_only: Some(true),
        ..Default::default()
    };
    let fmt = params.format;
    let fields_owned: Vec<String> = params.fields.to_vec();

    let response = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .list_meta(&api_params)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut envelope = serde_json::to_value(&response)
        .map_err(|e| TemperError::Api(format!("meta list serialize: {e}")))?;

    // Identity-out: every printed row carries its decorated `ref` (parity with the
    // full `list` path). Rows are `ResourceDetail` now, so they carry the title
    // `inject_ref` needs.
    if let Some(rows) = envelope.get_mut("rows").and_then(|r| r.as_array_mut()) {
        for row in rows.iter_mut() {
            inject_ref(row);
        }
    }

    // Truncation signal — parity with the full `list` path (see `inject_truncation_signal`).
    let total = envelope.get("total").and_then(|t| t.as_i64()).unwrap_or(0);
    let returned = envelope
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let truncated = inject_truncation_signal(&mut envelope, offset);

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
    if truncated {
        warn_truncated(total, returned);
    }
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

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            // A team is addressed by ref — a team UUID, a decorated `slug-<uuid>`, or a bare
            // slug — the same resolution `team show`/`context share` use. It is NOT a resource
            // ref, so it does not go through `parse_ref` (that yields a misleading
            // "not a resource ref" error for a valid slug — issue #366).
            let to_team_id = match to_team.as_deref() {
                Some(team) => Some(crate::actions::cogmap::resolve_team_id(client, team).await?),
                None => None,
            };
            let principal = crate::actions::cogmap::resolve_principal(to_profile, to_team_id)?;

            let body = temper_core::types::resource_grant::ResourceGrantBody {
                principal_table: principal.table,
                principal_id: principal.id,
                can_read: read || write || grant_cap,
                can_write: write,
                can_delete: false,
                can_grant: grant_cap,
            };

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

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            // `--from-team` is a team ref (UUID / decorated / bare slug), resolved the same way
            // as everywhere else on the team surface — not the resource-ref parser (issue #366).
            let from_team_id = match from_team.as_deref() {
                Some(team) => Some(crate::actions::cogmap::resolve_team_id(client, team).await?),
                None => None,
            };
            let principal = crate::actions::cogmap::resolve_principal(from_profile, from_team_id)?;

            let body = temper_core::types::resource_grant::ResourceRevokeBody {
                principal_table: principal.table,
                principal_id: principal.id,
            };

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
    // Only the row is needed here — `get` returns both meta tiers, which delete ignores.
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?
    .row;

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

/// Fold a resource's metadata, body, and its optional edge/provenance sections into
/// ONE JSON document.
///
/// `show` used to `println!` once per section, so `--edges` emitted two concatenated
/// JSON documents and `--provenance` a third — a single `json.load()` raised
/// `Extra data`. Building the composite here and printing once makes a multi-document
/// JSON response structurally impossible rather than merely test-detectable.
pub(crate) fn build_show_document(
    metadata: serde_json::Value,
    body: &str,
    edges: Option<EdgesReport>,
    lineage: Option<temper_core::types::lineage::ResourceLineage>,
    provenance: Option<Vec<temper_core::types::provenance::BlockProvenanceRow>>,
) -> Result<serde_json::Value> {
    let mut doc = metadata;
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| TemperError::Api("resource metadata is not a JSON object".to_string()))?;

    obj.insert(
        "content".to_string(),
        serde_json::Value::String(body.to_string()),
    );

    if let Some(edges) = edges {
        obj.insert(
            "edges".to_string(),
            serde_json::to_value(edges)
                .map_err(|e| TemperError::Api(format!("edges serialize: {e}")))?,
        );
    }

    if let Some(lineage) = lineage {
        obj.insert(
            "lineage".to_string(),
            serde_json::to_value(lineage)
                .map_err(|e| TemperError::Api(format!("lineage serialize: {e}")))?,
        );
    }

    if let Some(provenance) = provenance {
        obj.insert(
            "provenance".to_string(),
            serde_json::to_value(provenance)
                .map_err(|e| TemperError::Api(format!("provenance serialize: {e}")))?,
        );
    }

    Ok(doc)
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
            // `get` returns a `ResourceDetail`: the row flattened, plus both meta tiers.
            // The tiers are what make the full `show` a superset of `--meta-only`.
            let detail = client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client
                .resources()
                .content(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            // Per-resource projection refresh — best-effort. The projection writer takes the
            // row; the meta tiers reach the file via `resp`'s managed/open fields.
            if let Err(e) = crate::projection::write_resource_file_from_parts(
                &config_clone.vault_root,
                &detail.row,
                &resp,
            ) {
                crate::output::warning(format!("could not refresh projection file: {e}"));
            }

            let metadata = serde_json::to_value(&detail)
                .map_err(|e| TemperError::Api(format!("metadata serialize: {e}")))?;
            Ok((metadata, resp.markdown))
        })
    })?;

    inject_ref(&mut metadata);

    // Fetch every requested section BEFORE rendering: the JSON arm folds them into
    // one document, so nothing may be printed until all of them are in hand.
    let edges = if params.edges {
        Some(fetch_edges(id)?)
    } else {
        None
    };
    let lineage = if params.lineage {
        Some(fetch_lineage(id)?)
    } else {
        None
    };
    let provenance = if params.provenance {
        Some(fetch_provenance(id)?)
    } else {
        None
    };

    match params.format {
        crate::format::OutputFormat::Json => {
            let doc = build_show_document(metadata, &body, edges, lineage, provenance)?;
            let rendered = crate::format::render(&doc, params.format)?;
            crate::output::plain(rendered);
        }
        // Toon is the human TTY surface: keep the frontmatter+body document, then append
        // each requested section as its own block. The one-document contract is a JSON
        // (agent-surface) invariant, not a Toon one.
        crate::format::OutputFormat::Toon => {
            let rendered = crate::format::render_resource_show(&metadata, &body, params.format)?;
            crate::output::plain(rendered);
            if let Some(edges) = edges {
                crate::output::plain(crate::format::render(&edges, params.format)?);
            }
            if let Some(lineage) = lineage {
                crate::output::plain(crate::format::render(&lineage, params.format)?);
            }
            if let Some(provenance) = provenance {
                crate::output::plain(crate::format::render(&provenance, params.format)?);
            }
        }
    }

    Ok(())
}

/// Show a resource's evidential-standing shape.
///
/// Cloud-only and context-free: the ref resolves to a `ResourceId` (trailing-UUID-only,
/// via `parse_ref`, exactly as `show` does), the `StandingShape` is fetched by id from
/// `GET /api/resources/{id}/evidence`, and the whole struct is rendered through the
/// shared `format`/`output` helpers. The struct carries both the shape vector AND the
/// lossy `band` chip, so serializing it whole emits the band alongside the shape (spec
/// §1.1) — never in place of it. An unreadable/absent finding is a NotFound error.
pub fn evidence(_config: &Config, r#ref: &str, format: crate::format::OutputFormat) -> Result<()> {
    use crate::actions::runtime;

    let id = temper_workflow::operations::parse_ref(r#ref)?;

    let shape = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .evidence(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&shape, format)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `show --meta-only`: the full `show` view **minus the body**.
///
/// Fetches the same `ResourceDetail` the default path does (the row flattened
/// — title, doc_type, context, owner, the stage/seq/mode/effort projections —
/// plus both `managed_meta` and `open_meta` tiers), and simply skips the
/// separate `content` body fetch. So `--meta-only` is a strict subset of the
/// full `show`: everything except the (expensive-to-reconstruct) body. Applies
/// the shared top-level projection filter when `fields` is non-empty.
///
/// This deliberately hits `GET /api/resources/{id}` (row + tiers, no body
/// reconstruction) rather than the narrower `GET /api/resources/{id}/meta`:
/// the meta endpoint omits every identity/display field (title included), which
/// made the projection too lossy to orient from.
///
/// Cloud-only and context-free: the id was already resolved from the ref by
/// `show`; this calls `get` by id directly (no `resolve_by_uri`).
fn show_meta_only(
    _config: &Config,
    id: temper_core::types::ids::ResourceId,
    fmt: crate::format::OutputFormat,
    fields: &[String],
) -> Result<()> {
    use crate::actions::runtime;

    let fields_inner = fields.to_vec();

    let detail = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut value = serde_json::to_value(&detail)
        .map_err(|e| TemperError::Api(format!("meta serialize: {e}")))?;
    // Inject `ref` (and `context_ref`) before the `--fields` filter, exactly as the full
    // `show` and `list` do: the anchor `id` is always preserved. Now that `--meta-only`
    // carries the title, this produces the same decorated `ref` the full `show` emits.
    inject_ref(&mut value);
    let filtered = temper_core::projection::apply_top_level_filter(value, &fields_inner, "id")
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

/// Fetch a resource's graph edges, grouped by direction.
///
/// Cloud-only and context-free: the id was already resolved from the ref by
/// `show`. Returns data — `show` decides how to render it.
fn fetch_edges(id: temper_core::types::ids::ResourceId) -> Result<EdgesReport> {
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

    Ok(EdgesReport { outgoing, incoming })
}

/// Fetch a resource's bidirectional `derived_from` lineage via the API.
///
/// Hits `GET /api/resources/{id}/lineage` and returns ancestors + descendants,
/// each access-gated. An unreadable/absent resource is a NotFound error.
fn fetch_lineage(
    id: temper_core::types::ids::ResourceId,
) -> Result<temper_core::types::lineage::ResourceLineage> {
    use crate::actions::runtime;

    runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .lineage(uuid::Uuid::from(id), None)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })
}

/// Fetch the itemized per-block provenance for a resource via the API.
///
/// Hits `GET /api/resources/{id}/provenance` and returns the rows in
/// `(block, accretion)` order. An unreadable resource returns an empty list
/// (access-scoped in SQL).
fn fetch_provenance(
    id: temper_core::types::ids::ResourceId,
) -> Result<Vec<temper_core::types::provenance::BlockProvenanceRow>> {
    use crate::actions::runtime;

    runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .provenance(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })
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

/// Print the self-describing open_meta convention (recognized keys, shapes, FTS-indexing, and
/// discouraged keys). Mirrors the MCP `describe_open_meta` tool — both render the shared
/// [`temper_workflow::schema::OpenMetaConvention`]. Respects `--format`.
pub fn describe_open_meta(format: crate::format::OutputFormat) -> Result<()> {
    let convention = temper_workflow::schema::describe_open_meta()?;
    let rendered = crate::format::render(&convention, format)?;
    crate::output::plain(rendered);
    Ok(())
}

/// Send-side gate for the open (caller-defined) frontmatter tier (create + update), the twin of the
/// server's `validate_open_meta_shape` (symmetric defense — both ends inject/validate from the same
/// schema so a mis-shaped recognized key never reaches storage). Hard-errors on a recognized key
/// carrying the wrong shape (e.g. `descriptor: 42`, a malformed `date`) — for the FTS-indexed keys a
/// wrong shape stores-but-does-not-index, a silent search miss. Unrecognized keys always pass (the tier
/// is open), so version skew never hard-fails. Discouraged keys (bare `slug`/`title`, whose canonical
/// home is `temper-slug`/`temper-title`) surface as a non-blocking stderr warning — the write proceeds.
fn validate_open_meta_send_side(open_meta: &serde_json::Value) -> Result<()> {
    let issues = temper_workflow::schema::validate_open_meta(open_meta)?;
    if !issues.is_empty() {
        let detail = issues
            .iter()
            .map(|i| {
                let where_ = if i.path.is_empty() {
                    "open_meta"
                } else {
                    &i.path
                };
                format!("{where_}: {}", i.message)
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(TemperError::BadRequest(format!(
            "invalid --open-meta shape: {detail}. Run `temper resource describe-open-meta` for the \
             recognized conventions"
        )));
    }
    for warning in temper_workflow::schema::check_discouraged_open_meta_keys(open_meta) {
        output::warning(&warning.message);
    }
    Ok(())
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
        let merged = serde_json::Value::Object(obj);
        // Validate the merged open tier send-side (shape hard-error + discouraged-key warning); the
        // server re-enforces the shape gate. The typed list flags (`--tags`/…) are already well-shaped,
        // so in practice this catches a mis-shaped `--open-meta` JSON blob.
        validate_open_meta_send_side(&merged)?;
        Ok(Some(merged))
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
    // Update needs only the row (doctype validation); `get` also carries both meta tiers.
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?
    .row;
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
    let rendered = render_action_result_with_ref(&result, params.format)?;
    crate::output::plain(rendered);

    Ok(())
}

/// Args for [`annotate`] — the annotate-only provenance backfill (issue #355).
pub struct AnnotateParams<'a> {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub r#ref: &'a str,
    /// Provenance source refs/URLs (`--sources`) — resolved to `ProvenanceSource::Resource` (refs) or
    /// `ProvenanceSource::Remote` (URLs, locator fragment preserved). Non-empty (clap `required`).
    pub sources: &'a [String],
    /// Which content block to annotate (`--content-block`, a block UUID). `None` → the sole body block.
    pub content_block: Option<uuid::Uuid>,
    /// Output format, resolved globally upstream in `main`.
    pub format: crate::format::OutputFormat,
    /// Per-act correlation + authorship for the annotate act.
    pub act: temper_core::types::ActInput,
}

/// Attach provenance sources to a resource's block WITHOUT a body revise (issue #355).
///
/// The annotate-only counterpart to `update --sources`: it records block-provenance rows on the
/// addressed block with no re-chunk/re-embed (body_hash + embeddings unchanged). Verify the recorded
/// rows with `resource show --provenance`.
pub fn annotate(config: &Config, params: AnnotateParams<'_>) -> Result<()> {
    use temper_workflow::operations::AnnotateResource;

    // Resolve the ref to an id + fetch the current row (for its home context — build_backend needs it).
    let id = temper_workflow::operations::parse_ref(params.r#ref)?;
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?
    .row;

    // Resolve --sources refs → provenance records. A ref that fails to parse is a hard error
    // (escalate, never a silent drop). clap guarantees the list is non-empty (`required = true`).
    let resolved_sources = resolve_provenance_sources(params.sources)?;

    let cmd = AnnotateResource {
        resource: id,
        sources: resolved_sources,
        content_block: params.content_block,
        act: params.act.clone().into_act_context()?,
        origin: temper_workflow::operations::Surface::CliCloud,
    };

    let (runtime, backend, _client) = crate::backend_select::build_backend(
        config,
        row.context_name.as_deref().unwrap_or_default(),
    )?;
    let output = runtime.block_on(backend.annotate_resource(cmd))?;

    // The resource body is unchanged, so there is no projection file to rewrite — emit the same flat
    // action result `update` does (status + resource row), so the two write verbs read identically.
    let result = UpdateActionResult {
        status: "ok",
        resource: output.value,
    };
    let rendered = render_action_result_with_ref(&result, params.format)?;
    crate::output::plain(rendered);
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
mod list_helpers_tests {
    use super::*;
    use temper_workflow::types::resource::{ResourceSortField, SortOrder};

    #[test]
    fn parse_sort_field_aliases() {
        assert!(matches!(
            parse_sort_arg("updated").unwrap().0,
            ResourceSortField::Updated
        ));
        assert!(matches!(
            parse_sort_arg("created_at").unwrap().0,
            ResourceSortField::Created
        ));
        assert!(matches!(
            parse_sort_arg("context").unwrap().0,
            ResourceSortField::ContextName
        ));
        assert!(matches!(
            parse_sort_arg("doc-type").unwrap().0,
            ResourceSortField::DocTypeName
        ));
    }

    #[test]
    fn parse_sort_direction_defaults_per_field() {
        // Time/seq fields default to descending (newest/highest first).
        assert!(matches!(
            parse_sort_arg("updated").unwrap().1,
            SortOrder::Desc
        ));
        assert!(matches!(parse_sort_arg("seq").unwrap().1, SortOrder::Desc));
        // Textual fields default to ascending (A→Z).
        assert!(matches!(parse_sort_arg("title").unwrap().1, SortOrder::Asc));
        assert!(matches!(parse_sort_arg("stage").unwrap().1, SortOrder::Asc));
    }

    #[test]
    fn parse_sort_explicit_direction_overrides_default() {
        let (f, o) = parse_sort_arg("title:desc").unwrap();
        assert!(matches!(f, ResourceSortField::Title));
        assert!(matches!(o, SortOrder::Desc));
        let (_, o) = parse_sort_arg("updated:asc").unwrap();
        assert!(matches!(o, SortOrder::Asc));
    }

    #[test]
    fn parse_sort_rejects_unknown_field_and_direction() {
        // A bad field or direction is a hard error, never a silent mis-sort.
        assert!(parse_sort_arg("bogus").is_err());
        assert!(parse_sort_arg("title:sideways").is_err());
    }

    #[test]
    fn resolve_list_limit_all_means_no_cap() {
        assert_eq!(resolve_list_limit(true, None, 20), None);
        // `--all` wins over any (clap-excluded) limit.
        assert_eq!(resolve_list_limit(true, Some(5), 20), None);
    }

    #[test]
    fn resolve_list_limit_uses_explicit_then_default() {
        assert_eq!(resolve_list_limit(false, Some(5), 20), Some(5));
        assert_eq!(resolve_list_limit(false, None, 20), Some(20));
        assert_eq!(resolve_list_limit(false, None, 50), Some(50));
    }

    #[test]
    fn truncation_signal_flags_a_capped_page() {
        // 2 of 5 shown from offset 0 → truncated.
        let mut env = serde_json::json!({
            "rows": [{"id": "a"}, {"id": "b"}],
            "total": 5,
        });
        assert!(inject_truncation_signal(&mut env, 0));
        assert_eq!(env["truncated"], serde_json::json!(true));
        assert_eq!(env["returned"], serde_json::json!(2));
    }

    #[test]
    fn truncation_signal_clear_when_page_covers_the_tail() {
        // offset 3 + 2 returned == total 5 → nothing beyond this page.
        let mut env = serde_json::json!({
            "rows": [{"id": "d"}, {"id": "e"}],
            "total": 5,
        });
        assert!(!inject_truncation_signal(&mut env, 3));
        assert_eq!(env["truncated"], serde_json::json!(false));

        // Whole set on one page.
        let mut whole = serde_json::json!({
            "rows": [{"id": "a"}, {"id": "b"}],
            "total": 2,
        });
        assert!(!inject_truncation_signal(&mut whole, 0));
        assert_eq!(whole["truncated"], serde_json::json!(false));
    }
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

    #[test]
    fn origin_uri_from_url_source() {
        // A URL `--from` becomes the resource's origin (verbatim — casing preserved for the
        // display URL; the server normalizes only the dedup key).
        assert_eq!(
            origin_uri_from_source(Some("https://Example.com/issue/42"), false),
            Some("https://Example.com/issue/42".to_owned())
        );
        assert_eq!(
            origin_uri_from_source(Some("http://a.test/x"), false),
            Some("http://a.test/x".to_owned())
        );
    }

    #[test]
    fn origin_uri_none_for_local_path_source() {
        // A local `--from` path has no external origin.
        assert_eq!(origin_uri_from_source(Some("./notes/doc.pdf"), false), None);
        assert_eq!(origin_uri_from_source(Some("/abs/path.md"), false), None);
    }

    #[test]
    fn origin_uri_suppressed_by_no_source() {
        // `--no-source` opts out entirely, preserving the pre-#352 empty-origin behavior.
        assert_eq!(
            origin_uri_from_source(Some("https://example.com/x"), true),
            None
        );
    }

    #[test]
    fn origin_uri_none_when_no_from() {
        assert_eq!(origin_uri_from_source(None, false), None);
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
    fn validate_open_meta_send_side_passes_wellshaped_and_unknown_keys() {
        // Recognized keys with correct shapes + an unknown key (open tier stays open) all pass.
        let om = serde_json::json!({
            "tags": ["release", "infra"],
            "descriptor": "the full section descriptor",
            "date": "2026-07-11",
            "some_future_convention": {"nested": true}
        });
        assert!(validate_open_meta_send_side(&om).is_ok());
    }

    #[test]
    fn validate_open_meta_send_side_hard_errors_on_misshaped_recognized_key() {
        // descriptor must be a string; a number is a shape violation → hard error.
        assert!(validate_open_meta_send_side(&serde_json::json!({"descriptor": 42})).is_err());
        // date must match YYYY-MM-DD.
        assert!(validate_open_meta_send_side(&serde_json::json!({"date": "July 11"})).is_err());
        // tags items must be strings.
        assert!(validate_open_meta_send_side(&serde_json::json!({"tags": [1, 2]})).is_err());
    }

    #[test]
    fn validate_open_meta_send_side_warns_but_passes_on_discouraged_keys() {
        // A bare `slug` is discouraged (canonical home is temper-slug) but not a shape error, so the
        // write proceeds (warning goes to stderr).
        assert!(validate_open_meta_send_side(&serde_json::json!({"slug": "my-thing"})).is_ok());
    }

    #[test]
    fn build_open_meta_for_update_rejects_misshaped_merged_open_meta() {
        // A mis-shaped recognized key in the --open-meta blob is caught at merge time.
        let mut params = empty_update_params("foo");
        params.open_meta = Some(r#"{"descriptor": 42}"#);
        assert!(build_open_meta_for_update(&params).is_err());
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

    use super::{
        render_action_result_with_ref, CreateActionResult, DeleteActionResult, UpdateActionResult,
    };

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
            ingest_state: Some(temper_workflow::types::IngestState::Complete),
            body_storage: Some(temper_workflow::types::resource::BodyStorage::Derived),
        }
    }

    /// Task 9: `CreateActionResult` flattens `ResourceRow` — all wire-type
    /// `create` is a create-style response: it must carry the same decorated `ref` that
    /// `list`/`show`/`search` rows carry, so an agent can address the thing it just made
    /// without a second round-trip. It used to be the only one that didn't.
    #[test]
    fn render_create_action_result_carries_ref() {
        let row = make_resource_row("2026-05-14-test", "task", "Test Task", "temper");
        let result = CreateActionResult {
            status: "ok",
            resource: row,
            edges_asserted: Vec::new(),
            edges_failed: Vec::new(),
        };
        let out = render_action_result_with_ref(&result, crate::format::OutputFormat::Json)
            .expect("json render");
        let v: serde_json::Value = serde_json::from_str(&out).expect("exactly one json document");

        let r = v["ref"].as_str().expect("create response carries a `ref`");
        let id = v["id"].as_str().expect("id");
        assert!(
            r.starts_with("test-task-") && r.ends_with(id),
            "ref is the decorated `sluggify(title)-<uuid>` form: {out}"
        );
    }

    #[test]
    fn render_update_action_result_carries_ref() {
        let row = make_resource_row("2026-05-14-test", "task", "Test Task", "temper");
        let result = UpdateActionResult {
            status: "ok",
            resource: row,
        };
        let out = render_action_result_with_ref(&result, crate::format::OutputFormat::Json)
            .expect("json render");
        let v: serde_json::Value = serde_json::from_str(&out).expect("exactly one json document");
        assert!(
            v["ref"].as_str().is_some(),
            "update response carries a `ref`: {out}"
        );
    }

    /// fields appear at the top level alongside `status`. The old per-doctype
    /// `temper-slug` / `temper-title` keys must not appear.
    #[test]
    fn render_create_action_result_json_is_flat() {
        let row = make_resource_row("2026-05-14-test", "task", "Test Task", "temper");
        let result = CreateActionResult {
            status: "ok",
            resource: row,
            edges_asserted: Vec::new(),
            edges_failed: Vec::new(),
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
            edges_asserted: Vec::new(),
            edges_failed: Vec::new(),
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
    use std::io::Cursor;

    use super::*;

    /// Stdin-readiness probe stand-in: input is ready to read (a genuine pipe, or EOF).
    fn ready() -> bool {
        true
    }
    /// Stdin-readiness probe stand-in: stdin is open but idle (no input ready).
    fn idle() -> bool {
        false
    }

    #[tokio::test]
    async fn from_and_body_are_mutually_exclusive() {
        // resolve_from_input errors when both --from and --body are provided.
        let err = resolve_from_input(
            Some("/tmp/x.md"),
            Some("@body.md"),
            true,
            Cursor::new(b""),
            ready,
        )
        .await
        .expect_err("should error on mutex");
        assert!(
            format!("{err}").contains("--from cannot be combined with --body"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn from_with_a_genuinely_piped_body_errors() {
        // A non-TTY stdin that actually carries bytes IS a real --from/--body collision.
        let err = resolve_from_input(
            Some("/tmp/x.md"),
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# piped body"),
            ready,
        )
        .await
        .expect_err("should error on a real piped body");
        assert!(format!("{err}").contains("piped stdin body"), "got: {err}");
    }

    #[tokio::test]
    async fn from_with_idle_non_tty_stdin_is_allowed() {
        // The issue #420 item 1 regression: an open-but-idle non-TTY stdin (the agent/CI
        // case) must NOT be treated as a conflict. The probe reports not-ready, so stdin is
        // never read — we fall through to the path check (which errors for a different,
        // expected reason, proving the stdin gate was passed).
        let err = resolve_from_input(
            Some("/tmp/definitely_does_not_exist_420.md"),
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# would block in prod / must be ignored"),
            idle,
        )
        .await
        .expect_err("should reach the path check");
        assert!(
            format!("{err}").contains("--from path does not exist"),
            "idle non-TTY stdin must pass the gate, not error as a conflict; got: {err}"
        );
    }

    #[tokio::test]
    async fn from_with_eof_stdin_is_allowed() {
        // `< /dev/null`: the probe reports ready (EOF), but the read drains to empty, so it is
        // not a conflict. This is the exact case the issue calls out as wrongly rejected.
        let err = resolve_from_input(
            Some("/tmp/definitely_does_not_exist_420.md"),
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b""),
            ready,
        )
        .await
        .expect_err("should reach the path check");
        assert!(
            format!("{err}").contains("--from path does not exist"),
            "empty (EOF) non-TTY stdin must pass the gate; got: {err}"
        );
    }

    #[tokio::test]
    async fn from_file_uri_resolves_to_a_local_file() {
        // `--from` is forgiving about the file:// spelling: it decodes to the local path
        // (percent-escapes included — note the space in the filename) and reads it like any
        // other local file, converging with the plain-path branch.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("from source.md");
        std::fs::write(&file, "# hello from a file uri").unwrap();
        let uri = url::Url::from_file_path(&file).unwrap().to_string();
        assert!(
            uri.starts_with("file://") && uri.contains("%20"),
            "sanity: {uri}"
        );

        let body = resolve_from_input(Some(&uri), None, true, Cursor::new(b""), ready)
            .await
            .expect("file:// URI should resolve to the local file")
            .expect("should return a body");
        assert!(body.contains("hello from a file uri"), "got: {body}");
    }

    #[tokio::test]
    async fn from_a_pdf_with_no_text_layer_refuses_rather_than_ingesting_nothing() {
        // A scanned / image-only PDF is structurally valid, so the extractor opens it happily and
        // returns Ok("") — it has no text to give. That empty body used to flow straight through:
        // it was filtered to None and the backend synthesized `# {title}` in its place, so the
        // command exited 0, printed a ref, and stored a title-only resource. The document was
        // silently gone. Refuse instead — a knowledge base must not swallow a document (#420).
        //
        // The fixture is a valid one-page PDF with no text operators. It must fail HERE, on the
        // empty extraction — not as a parse error, which would make this test pass for the wrong
        // reason.
        let pdf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/no-text-layer.pdf");
        let err = resolve_from_input(
            Some(pdf.to_str().unwrap()),
            None,
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
            ready,
        )
        .await
        .expect_err("a PDF with no text layer must not ingest as an empty body");

        let msg = format!("{err}");
        assert!(
            msg.contains("no text"),
            "must say the PDF had no text to extract, got: {msg}"
        );
        assert!(
            msg.contains("no-text-layer.pdf"),
            "must name the offending file, got: {msg}"
        );
        assert!(
            msg.contains("ocrmypdf"),
            "must point at a way forward (OCR), got: {msg}"
        );
    }

    #[tokio::test]
    async fn from_missing_file_uri_reports_path_not_found() {
        // A file:// URI that resolves to a non-existent path gets the normal path-not-found
        // error — not a bespoke "file:// not accepted" rejection.
        let err = resolve_from_input(
            Some("file:///tmp/definitely_does_not_exist_420.md"),
            None,
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
            ready,
        )
        .await
        .expect_err("should error on the missing resolved path");
        assert!(format!("{err}").contains("does not exist"), "got: {err}");
    }

    #[tokio::test]
    async fn from_path_does_not_exist_errors() {
        // resolve_from_input errors when the path doesn't exist.
        let err = resolve_from_input(
            Some("/tmp/definitely_does_not_exist_ch7.md"),
            None,
            true,
            Cursor::new(b""),
            ready,
        )
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
            ingest_state: Some(temper_workflow::types::IngestState::Complete),
            body_storage: Some(temper_workflow::types::resource::BodyStorage::Derived),
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
        // Build a stub meta-list envelope. Rows are `ResourceDetail`-shaped now
        // (full row + both tiers), so they carry `title`/`doc_type_name` too.
        let envelope = serde_json::json!({
            "rows": [
                {
                    "id": "11111111-1111-1111-1111-111111111111",
                    "title": "Alpha",
                    "doc_type_name": "task",
                    "managed_meta": {"stage": "in-progress"},
                    "open_meta": {"tags": []}
                },
                {
                    "id": "22222222-2222-2222-2222-222222222222",
                    "title": "Beta",
                    "doc_type_name": "task",
                    "managed_meta": {"stage": "done"},
                    "open_meta": null
                }
            ],
            "total": 2,
            "facets": {"doc_type": {"task": 2}}
        });

        // Filter the rows array (the action layer will apply the filter
        // to envelope.rows specifically, not to the whole envelope).
        let rows = envelope.get("rows").cloned().expect("rows");
        let filtered_rows =
            apply_top_level_filter(rows, &["managed_meta".to_string()], "id").expect("filter");

        // Each row should have only id + managed_meta
        let arr = filtered_rows.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for row in arr {
            assert!(row.get("id").is_some(), "anchor missing in {row}");
            assert!(
                row.get("managed_meta").is_some(),
                "managed_meta missing in {row}"
            );
            assert!(
                row.get("open_meta").is_none(),
                "open_meta should be dropped"
            );
            assert!(
                row.get("title").is_none(),
                "title should be dropped by --fields managed_meta"
            );
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

    /// A titleless row (the `--meta-only` projection) gets no `ref` rather than a
    /// fabricated `-<uuid>` one. Surfaced by the #330 differential e2e: the malformed ref
    /// made `--meta-only` disagree with the full `show` on the same key.
    #[test]
    fn inject_ref_skips_rows_without_a_title() {
        let mut row = serde_json::json!({
            "id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "managed_meta": {},
        });
        super::inject_ref(&mut row);
        assert!(
            row.get("ref").is_none(),
            "no title means no decorated ref: {row}"
        );
    }
}

#[cfg(test)]
mod show_meta_only_tests {
    use temper_core::projection::apply_top_level_filter;
    use temper_workflow::types::managed_meta::{ManagedMeta, ResourceMetaResponse};

    fn fake_meta_response() -> ResourceMetaResponse {
        ResourceMetaResponse {
            id: temper_core::types::ResourceId::from(uuid::Uuid::nil()),
            managed_meta: Some(ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..Default::default()
            }),
            open_meta: Some(serde_json::json!({"tags": ["x"]})),
        }
    }

    #[test]
    fn show_meta_only_fields_filter_preserves_anchor_and_managed_meta_only() {
        let response = fake_meta_response();
        let value = serde_json::to_value(&response).expect("serialize");
        let filtered =
            apply_top_level_filter(value, &["managed_meta".to_string()], "id").expect("filter");
        assert!(filtered.get("id").is_some(), "anchor missing");
        assert!(
            filtered.get("managed_meta").is_some(),
            "managed_meta missing"
        );
        assert!(
            filtered.get("open_meta").is_none(),
            "open_meta should be filtered out"
        );
    }

    #[test]
    fn show_meta_only_no_fields_returns_full_response() {
        let response = fake_meta_response();
        let value = serde_json::to_value(&response).expect("serialize");
        let unfiltered = apply_top_level_filter(value.clone(), &[], "id").expect("filter");
        assert_eq!(unfiltered, value);
    }
}

/// Tests for `build_show_document` — the pure builder that folds `--edges`
/// and `--provenance` sections into the resource's JSON document so `show`
/// prints exactly once. See PR #330: `--edges`/`--provenance` used to each
/// print their own JSON document, so a single `json.load()` raised
/// `Extra data`.
#[cfg(test)]
mod build_show_document_tests {
    use super::{build_show_document, EdgesReport};

    #[test]
    fn build_show_document_folds_edges_and_provenance_into_one_object() {
        let metadata = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "title": "A Node",
        });
        let edges = EdgesReport {
            outgoing: vec![],
            incoming: vec![],
        };
        let lineage = temper_core::types::lineage::ResourceLineage {
            resource_id: uuid::Uuid::nil(),
            ancestors: vec![],
            descendants: vec![],
        };

        let doc = build_show_document(
            metadata,
            "# body\n",
            Some(edges),
            Some(lineage),
            Some(vec![]),
        )
        .expect("build show document");

        // One document: content, edges, lineage, and provenance all hang off the resource object.
        assert_eq!(doc["title"], "A Node");
        assert_eq!(doc["content"], "# body\n");
        assert!(doc["edges"]["outgoing"].is_array(), "edges folded: {doc}");
        assert!(doc["edges"]["incoming"].is_array(), "edges folded: {doc}");
        assert!(
            doc["lineage"]["ancestors"].is_array(),
            "lineage folded: {doc}"
        );
        assert!(
            doc["lineage"]["descendants"].is_array(),
            "lineage folded: {doc}"
        );
        assert!(doc["provenance"].is_array(), "provenance folded: {doc}");

        // And it round-trips through a single `serde_json::from_str` with no trailing data.
        let rendered = serde_json::to_string_pretty(&doc).expect("render");
        let _: serde_json::Value = serde_json::from_str(&rendered).expect("exactly one document");
    }

    #[test]
    fn build_show_document_omits_absent_sections() {
        let metadata = serde_json::json!({ "id": "11111111-1111-1111-1111-111111111111" });
        let doc =
            build_show_document(metadata, "b", None, None, None).expect("build show document");

        assert_eq!(doc["content"], "b");
        assert!(
            doc.get("edges").is_none(),
            "no edges key when not requested: {doc}"
        );
        assert!(
            doc.get("lineage").is_none(),
            "no lineage key when not requested: {doc}"
        );
        assert!(
            doc.get("provenance").is_none(),
            "no provenance key when not requested: {doc}"
        );
    }
}

#[cfg(test)]
mod source_edge_targets_tests {
    use super::source_edge_targets;

    #[test]
    fn source_edge_targets_selects_only_resource_sources() {
        use temper_core::types::provenance::ProvenanceSource;

        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        let sources = vec![
            ProvenanceSource::Resource(a),
            ProvenanceSource::Remote("https://example.com/post".to_string()),
            ProvenanceSource::Resource(b),
            ProvenanceSource::Event(uuid::Uuid::from_u128(3)),
        ];

        let targets = source_edge_targets(&sources);

        // Remote URLs and event ids have no resource target — they cannot become edges.
        assert_eq!(targets, vec![a, b]);
    }

    #[test]
    fn source_edge_targets_is_empty_without_resource_sources() {
        use temper_core::types::provenance::ProvenanceSource;
        let sources = vec![ProvenanceSource::Remote("https://x.test".to_string())];
        assert!(source_edge_targets(&sources).is_empty());
    }
}
