use chrono::Local;
use temper_core::types::resource_view::{ResourceSection, SectionSet};
use temper_workflow::schema;

use crate::commands::resource_sections;
use crate::config::Config;
use crate::error::{Result, TemperError};
use crate::output;
use crate::vault;

/// Flat result emitted by `temper resource create`.
///
/// `ResourceView` is flattened so all wire-type fields appear at the top level
/// alongside `status`. Breaking change (Task 9): replaces the 7-variant
/// per-doctype JSON shape map (Task/Goal/Session/Research/Concept/Decision/default).
#[derive(Debug, serde::Serialize)]
pub(crate) struct CreateActionResult {
    pub status: &'static str,
    #[serde(flatten)]
    pub resource: temper_core::types::resource_view::ResourceView,
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
    pub resource: temper_core::types::resource_view::ResourceView,
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
/// Reads the anchor id from `id` (ResourceView) OR `resource_id`
/// (UnifiedSearchResultRow, which still
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
    // A row carrying no `title` cannot form the decorated half of a ref. No read path emits
    // one today (the retired `--meta-only` projection was the one that did), so this arm is
    // a guard rather than a live case. It used to default the title to `""` and
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

/// Insert a derived `ref` key into a serialized cogmap row (`CogmapRow`), computed from `id` + `name`
/// via `decorated_ref` (`sluggify(name)-<uuid>`). Render-time only — never persisted, never on the
/// wire type. No-op if `id`/`name` are absent or `id` is unparseable. Cogmap refs resolve
/// trailing-UUID-only, so the slug half is a copy-pasteable, self-documenting decoration.
pub(crate) fn inject_cogmap_ref(row: &mut serde_json::Value) {
    if let Some(obj) = row.as_object_mut() {
        let id = obj.get("id").and_then(|v| v.as_str()).map(str::to_owned);
        let name = obj.get("name").and_then(|v| v.as_str()).map(str::to_owned);
        if let (Some(id), Some(name)) = (id, name) {
            if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
                let decorated = temper_workflow::operations::decorated_ref(
                    &name,
                    temper_core::types::ids::ResourceId(uuid),
                );
                obj.insert("ref".to_string(), serde_json::Value::String(decorated));
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
    let open_meta_value = open_meta
        .map(|raw| parse_open_meta_flag("--open-meta", raw))
        .transpose()?;
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
        // Mint an owner-scoped create idempotency key up front (issue #581, spike rung 3-C). Every
        // CLI create carries one so a transient-failure retry — the in-process HTTP retry loop, or a
        // segmented resume that replays the persisted key — converges on the already-committed
        // resource via `(owner, key)` dedup instead of minting a duplicate. UUIDv7 by convention
        // (time-sortable, like every other id this repo mints); the server never treats it as a
        // resource id. The segmented path may replace this with a resumed key it persisted on a
        // prior attempt (see `run_segmented_create`).
        idempotency_key: Some(uuid::Uuid::now_v7()),
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
    let created_resource: temper_core::types::resource_view::ResourceView =
        runtime.block_on(backend.create_resource(cmd))?.value;

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
    /// `--with <section>[,…]`: sections to add on top of [`resource_sections::show_defaults`].
    pub with: &'a [String],
    /// `--without <section>[,…]`: sections to drop. A section named in both is a hard error.
    pub without: &'a [String],
    pub fields: &'a [String],
}

/// Parameters for the public `list` command, bundled to keep the CLI entry
/// signature compact (and clippy happy).
#[derive(Debug, Clone, Copy)]
pub struct ListParams<'a> {
    /// `--type`: optional. `None` lists across every doc type. It was mandatory until the
    /// tag filter landed, which made it untenable: tags span 14 doc types in production, so
    /// forcing a type meant enumerating one axis took 14 calls plus prior knowledge of which
    /// types exist — a client-side scan wearing a filter's clothes. The API and MCP had
    /// always taken `doc_type_name` as optional; this closes the gap rather than opening one.
    pub doc_type: Option<&'a str>,
    /// `--tag` (repeatable): resources carrying EVERY listed tag (AND), matched exactly and
    /// case-insensitively. Empty = no tag filter. Joined to the CSV the list endpoint's
    /// `tags` query param expects. Deliberately NOT doc-type-scoped — see `doc_type`.
    pub tag: &'a [String],
    pub context: Option<&'a str>,
    /// `--cogmap` (repeatable): scope to resources homed in these cognitive maps (UUID or decorated
    /// refs). Mutually exclusive with `context`. Empty = no cogmap filter.
    pub cogmap: &'a [String],
    pub limit: Option<usize>,
    /// `--all`: return every matching row (no page cap). Overrides `limit`
    /// (clap makes the two mutually exclusive, so both are never set together).
    pub all: bool,
    /// `--offset`: skip the first N matching rows (pagination).
    pub offset: Option<usize>,
    /// `--page`: the same axis counted in pages, 1-indexed. Clap makes it mutually
    /// exclusive with both `offset` and `all`, so at most one of the two is ever set and
    /// a paged call always has a page size to count in.
    pub page: Option<usize>,
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
    /// `--with <section>[,…]`: sections to fill on every row, on top of
    /// [`resource_sections::list_defaults`] (which asks for none). `body` is not offered.
    pub with: &'a [String],
    /// `--without <section>[,…]`: sections to drop. A section named in both is a hard error.
    pub without: &'a [String],
    pub fields: &'a [String],
}

/// The page size `list` uses when neither `--limit` nor `--all` is given.
///
/// **A default, not a cap.** An explicit `--limit` is honoured unchanged and no server-side
/// clamp exists (`crates/temper-api/src/handlers/resources.rs`), so this number bounds only
/// the call that asked for nothing. Kept small enough to be cheap, large enough that the
/// common case fits — and the `total`/`returned`/`truncated` trio makes any page
/// self-evident, so an agent never has to guess whether it saw the whole set.
///
/// **The one default.** There used to be a second, `DEFAULT_META_LIST_LIMIT = 50`, on the
/// `--meta-only` path, justified by meta rows being cheaper. That justification died with the
/// row types: `list` and `list --with open-meta` return the same `ResourceView`, differing by
/// one optional tier, so two page sizes meant a caller's page silently changed size when they
/// asked for a section. Worse, it is the number `--page` counts in — two defaults would make
/// `--page 3` mean row 40 or row 100 depending on an unrelated flag.
const DEFAULT_LIST_LIMIT: usize = 20;

/// Resolve repeated `--cogmap` refs (trailing-UUID-only) into the comma-separated UUID string the
/// list endpoint's `cogmap_ids` query param expects (the GET can't carry a `Vec`). `None` when no
/// `--cogmap` was given. Guards mutual exclusion with `--context`: a resource has exactly one home,
/// so the two filters could only ever intersect to the empty set — reject before the round-trip.
fn resolve_cogmap_scope_csv(cogmaps: &[String], context: Option<&str>) -> Result<Option<String>> {
    if cogmaps.is_empty() {
        return Ok(None);
    }
    if context.is_some() {
        return Err(TemperError::BadRequest(
            "--context and --cogmap are mutually exclusive; specify at most one home".into(),
        ));
    }
    let mut ids = Vec::with_capacity(cogmaps.len());
    for r in cogmaps {
        let id = temper_workflow::operations::parse_ref(r)
            .map_err(|e| TemperError::Config(format!("invalid cogmap ref {r:?}: {e}")))?;
        ids.push(id.0.to_string());
    }
    Ok(Some(ids.join(",")))
}

/// Refuse a type-scoped filter paired with a `--type` it cannot apply to. All three are sent
/// to the server unconditionally, so "ignored" was never true of any of them — a mismatch is
/// a hard error rather than a hint.
///
/// The guard fires **only when a type was actually named**. With `--type` omitted there is no
/// mismatch to report: `--stage backlog` alone honestly narrows to backlog tasks, because only
/// tasks carry a stage. Refusing that would be inventing a conflict the caller never stated.
/// The refusal exists for `--type goal --stage backlog`, where the caller has named a type the
/// flag demonstrably cannot apply to.
///
/// `--tag` is deliberately absent: tags are not doc-type-scoped (14 doc types carry them in
/// production), so there is no doc type it could mismatch.
fn check_type_scoped_filters(
    doc_type: Option<&str>,
    stage: Option<&str>,
    goal: Option<&str>,
    status: Option<&str>,
) -> Result<()> {
    let Some(doc_type) = doc_type else {
        return Ok(());
    };
    if stage.is_some() && doc_type != "task" {
        return Err(mismatched_filter_err("--stage", "task", doc_type));
    }
    if goal.is_some() && doc_type != "task" {
        return Err(mismatched_filter_err("--goal", "task", doc_type));
    }
    if status.is_some() && doc_type != "goal" {
        return Err(mismatched_filter_err("--status", "goal", doc_type));
    }
    Ok(())
}

/// Join repeated `--tag` values into the comma-separated string the list endpoint's `tags`
/// query param expects (the GET can't carry a `Vec`, same constraint as `--cogmap`). `None`
/// when no `--tag` was given.
///
/// A tag containing a comma is a hard error rather than a silent split. The transport cannot
/// express it, so `--tag "a,b"` would arrive server-side as two tags and quietly return a
/// different (narrower) set than the caller asked for — the exact shape of defect this filter's
/// goal exists to eliminate. No tag in the corpus contains a comma, so this refuses a value
/// nothing can currently be tagged with; it is a guard against the vocabulary growing one, not
/// a restriction on today's.
fn resolve_tag_csv(tags: &[String]) -> Result<Option<String>> {
    if tags.is_empty() {
        return Ok(None);
    }
    for t in tags {
        if t.contains(',') {
            return Err(TemperError::BadRequest(format!(
                "invalid --tag {t:?}: a tag may not contain a comma (the list filter is \
                 comma-separated over the wire, so it would silently split into two tags)"
            )));
        }
        if t.trim().is_empty() {
            return Err(TemperError::BadRequest(
                "invalid --tag: an empty tag matches nothing; omit the flag instead".into(),
            ));
        }
    }
    Ok(Some(tags.join(",")))
}

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
/// `--limit`, falling back to [`DEFAULT_LIST_LIMIT`].
///
/// The fallback is read from the constant rather than passed in: a `default` parameter is
/// what let two page sizes exist, and the caller that chose between them is gone.
fn resolve_list_limit(all: bool, limit: Option<usize>) -> Option<i64> {
    if all {
        None
    } else {
        Some(limit.unwrap_or(DEFAULT_LIST_LIMIT) as i64)
    }
}

/// Resolve the row offset a list call starts at, from `--offset` or `--page`.
///
/// `--offset` is passed through; `--page` is 1-indexed and multiplied by the **effective**
/// page size — the explicit `--limit` when there is one, else [`DEFAULT_LIST_LIMIT`]. That
/// is the whole subtlety: resolving `--page` against a hardcoded 20 would make
/// `--page 3 --limit 5` start at row 40 instead of row 10, silently skipping 30 rows and
/// returning a page the caller would have no reason to distrust.
///
/// Clap makes `--page` mutually exclusive with `--offset` and with `--all`, so the two arms
/// cannot both apply and an uncapped page never reaches the multiplication.
fn resolve_list_offset(page: Option<usize>, offset: Option<usize>, limit: Option<usize>) -> usize {
    match page {
        // `saturating_sub`, not `- 1`: clap's `range(1..)` makes page 0 unreachable, and this
        // keeps an unreachable input from being an arithmetic panic rather than encoding a
        // second opinion about what page 0 means.
        Some(page) => page.saturating_sub(1) * limit.unwrap_or(DEFAULT_LIST_LIMIT),
        None => offset.unwrap_or(0),
    }
}

/// Serialize a list response into the envelope the CLI prints: every row decorated with its
/// `ref`, then the optional `--fields` projection applied per row.
///
/// **`returned` and `truncated` are NOT computed here.** They arrive on the wire
/// (`ResourceListResponse`), derived by the server from the page it actually built. The CLI
/// used to inject them — `inject_truncation_signal` recomputed `offset + returned < total`
/// from the serialized JSON — which made the client a second, independent implementation of
/// a rule the server already applies, reachable only through the CLI. MCP callers got
/// neither key. Reading them off the response is what makes the signal a property of the
/// answer rather than of the surface that rendered it.
///
/// Pure, so the rendering half of `list` is testable without a server: `list` itself returns
/// `Result<()>` and prints, so nothing downstream of it can be observed.
fn build_list_envelope(
    response: &temper_workflow::types::resource::ResourceListResponse,
    fields: &[String],
) -> Result<serde_json::Value> {
    let mut envelope = serde_json::to_value(response)
        .map_err(|e| TemperError::Api(format!("list serialize: {e}")))?;

    // Identity-out: every printed row carries its decorated `ref`.
    if let Some(rows) = envelope.get_mut("rows").and_then(|r| r.as_array_mut()) {
        for row in rows.iter_mut() {
            inject_ref(row);
        }
    }

    if !fields.is_empty() {
        let rows = envelope
            .get_mut("rows")
            .ok_or_else(|| TemperError::Api("response missing `rows` envelope key".into()))?
            .take();
        let filtered_rows = temper_core::projection::apply_top_level_filter(rows, fields, "id")
            .map_err(map_projection_error)?;
        envelope["rows"] = filtered_rows;
    }

    Ok(envelope)
}

/// Emit the stderr note shown when a `list` page is truncated. Routed through
/// `output::warning` rather than `output::hint` for *severity*, not for stream
/// choice — both now write to stderr, so neither can corrupt the JSON document
/// an agent parses on stdout. A capped page an agent silently mistakes for the
/// whole set is a wrong answer, not a suggestion. Names the exact escape
/// hatches (`--all`, a bigger `--limit`, `--page`/`--offset`, or narrowing with
/// `--sort`/filters) so an agent self-corrects instead of asserting a set is
/// complete from a capped page.
///
/// Both arguments now come off `ResourceListResponse` rather than being counted from the
/// serialized rows, so the number a human reads and the `truncated` flag an agent reads are
/// the same server-side facts. `returned` is `i64` for that reason — it is the wire field,
/// not a `rows.len()`.
fn warn_truncated(total: i64, returned: i64) {
    output::warning(format!(
        "Showing {returned} of {total} matching results — the list is TRUNCATED. \
         Do not conclude a resource is absent or a set is complete from this page. \
         Re-run with --all (or a larger --limit, or --page/--offset to walk), or narrow \
         with --title-contains/--stage/--sort first."
    ));
}

/// A type-scoped filter was combined with a doc type it cannot apply to.
///
/// These three combinations used to emit a hint saying the filter was "ignored for
/// {type}" — and then send it anyway, so `--type goal --stage backlog` printed
/// *"ignored for goal"* above `total: 0` while 48 goals existed. The word was false in
/// the most expensive direction: a caller reading it attributes the empty page to
/// "nothing matches" rather than to the filter they were told was inert.
///
/// A hard error rather than a genuinely-ignored flag, because silently dropping an
/// explicit filter is the same class of lie one layer over: the caller asked to narrow
/// and got an unnarrowed set back with no way to tell.
fn mismatched_filter_err(flag: &str, only_for: &str, actual: &str) -> TemperError {
    TemperError::Project(format!(
        "{flag} applies only to --type {only_for}, but --type {actual} was given. \
         Drop {flag}, or list --type {only_for}."
    ))
}

/// Send-side half of the `--status` validation.
///
/// The predicate itself is `schema::validate_goal_status` — shared with
/// `substrate_read`'s receive-side check so the two ends cannot disagree about which
/// statuses exist. Checking here as well buys a faster, friendlier failure (no round
/// trip), not a second opinion.
///
/// Rejection is half the fix and the more important half: the defect's signature was
/// *accepting everything* (`--status bogus-value` returned all 43 goals), not returning
/// a wrong subset.
fn validate_status_filter(value: &str) -> Result<()> {
    schema::validate_goal_status(value)
}

/// Build the wire params both list endpoints take from the CLI's own `ListParams`.
///
/// `list` and `list --with open-meta` differ only in their default page cap and the `sections`
/// request, and until 2026-07-29 each carried a character-identical copy of this block. The tag
/// filter had to be added to both — which is exactly the drift the project's "never duplicate
/// filter logic; the two copies will drift" rule names.
///
/// **This is also the only place a `--flag` becomes a wire field, and the only place that is
/// observable from a test.** `list` returns `Result<()>` and prints its rows with `println!`,
/// so nothing downstream can see whether an assignment happened: a
/// `let _ = resolve_tag_csv(params.tag)?;` used to pass the entire suite, for `--tag` and for
/// every sibling filter. `every_list_flag_reaches_its_api_field` fails when any one of them
/// stops landing in its field.
fn list_api_params(
    params: &ListParams<'_>,
    sections: &SectionSet,
) -> Result<temper_workflow::types::resource::ResourceListParams> {
    let (sort, order) = match params.sort {
        Some(raw) => {
            let (f, o) = parse_sort_arg(raw)?;
            (Some(f), Some(o))
        }
        None => (None, None),
    };
    // Resolve --goal ref → goal resource id (trailing-UUID-only); the server filters on the live
    // `advances`→goal edge. An unparseable ref is a hard error (never a silent drop).
    let goal = params
        .goal
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(uuid::Uuid::from);
    Ok(temper_workflow::types::resource::ResourceListParams {
        doc_type_name: params.doc_type.map(ToString::to_string),
        context_ref: params.context.map(ToString::to_string),
        cogmap_ids: resolve_cogmap_scope_csv(params.cogmap, params.context)?,
        tags: resolve_tag_csv(params.tag)?,
        stage: params.stage.map(str::to_string),
        status: params.status.map(str::to_string),
        q: params.title_contains.map(str::to_string),
        goal,
        sort,
        order,
        limit: resolve_list_limit(params.all, params.limit),
        offset: Some(resolve_list_offset(params.page, params.offset, params.limit) as i64),
        // One envelope either way — the section is what varies, not the row type. Rendered
        // through `SectionSet::to_csv` rather than from a `contains` check per section, so a
        // section added to the vocabulary rides the wire instead of being silently dropped.
        sections: sections.to_csv(),
        ..Default::default()
    })
}

/// List resources of a given type (unified pipeline for all doc types).
///
/// **One path, whatever sections are asked for.** There used to be two — `list` and
/// `list_meta_only` — character-identical but for the client method they called and their
/// default page cap, because `--meta-only` selected a second *response type*. It selects a
/// *part* now (`?sections=open-meta` over the same `ResourceListResponse`), and the second
/// page cap went with it, so the branch has nothing left to branch on.
///
/// The paging state it prints (`returned`, `truncated`, `limit`, `offset`) is read off the
/// response, never recomputed here — see `build_list_envelope`.
pub fn list(_config: &Config, params: ListParams<'_>) -> Result<()> {
    check_type_scoped_filters(params.doc_type, params.stage, params.goal, params.status)?;

    if let Some(s) = params.stage {
        vault::validate_stage(s)?;
    }
    if let Some(s) = params.status {
        validate_status_filter(s)?;
    }

    let sections = resource_sections::resolve_sections(
        params.with,
        params.without,
        resource_sections::list_defaults(),
    )?;

    use crate::actions::runtime;

    let fmt = params.format;
    let api_params = list_api_params(&params, &sections)?;

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

    let envelope = build_list_envelope(&response, params.fields)?;

    // When truncated, inject a diagnostics array into the envelope so an agent
    // parsing stdout JSON can detect it without scraping stderr. The stderr
    // warning still fires for TTY/TOON humans.
    let envelope = if response.truncated {
        inject_truncation_diagnostic(envelope, response.total, response.returned)
    } else {
        envelope
    };

    let rendered = crate::format::render(&envelope, fmt)?;
    println!("{rendered}");
    if response.truncated {
        warn_truncated(response.total, response.returned);
    }
    Ok(())
}

/// Inject a `diagnostics` array into a `serde_json::Value` list envelope when
/// the response is truncated. Additive — absent when not truncated, present
/// with one warning entry when it is.
fn inject_truncation_diagnostic(
    mut envelope: serde_json::Value,
    total: i64,
    returned: i64,
) -> serde_json::Value {
    if let Some(obj) = envelope.as_object_mut() {
        obj.insert(
            "diagnostics".to_string(),
            serde_json::json!([{
                "level": "warning",
                "code": "truncated",
                "message": format!(
                    "Showing {returned} of {total} matching results — the list is TRUNCATED. \
                     Do not conclude a resource is absent or a set is complete from this page."
                ),
                "hint": "Re-run with --all (or a larger --limit, or --page/--offset to walk), \
                         or narrow with --title-contains/--stage/--sort first."
            }]),
        );
    }
    envelope
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

/// Fold a resource's metadata, body, and its optional edge/provenance sections into
/// ONE JSON document.
///
/// `show` used to `println!` once per section, so `--edges` emitted two concatenated
/// JSON documents and `--provenance` a third — a single `json.load()` raised
/// `Extra data`. Building the composite here and printing once makes a multi-document
/// JSON response structurally impossible rather than merely test-detectable.
pub(crate) fn build_show_document(
    metadata: serde_json::Value,
    body: Option<&str>,
    edges: Option<EdgesReport>,
    lineage: Option<temper_core::types::lineage::ResourceLineage>,
    provenance: Option<Vec<temper_core::types::provenance::BlockProvenanceRow>>,
) -> Result<serde_json::Value> {
    let mut doc = metadata;
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| TemperError::Api("resource metadata is not a JSON object".to_string()))?;

    // `None` omits the key rather than emitting `""` — the same distinction `ResourceView`
    // draws with `content: Option<String>`, where absent means "not requested" and never
    // "the body is empty". A `--without body` read that emitted `"content": ""` would be
    // indistinguishable from a resource whose body really is empty.
    if let Some(body) = body {
        obj.insert(
            "content".to_string(),
            serde_json::Value::String(body.to_string()),
        );
    }

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
/// Cloud-only and context-free: the ref resolves to a `ResourceId`, the view +
/// content are fetched by id (no `resolve_by_uri`, no doctype dispatch — the
/// three former per-doctype shows rendered identically), the canonical
/// projection file is refreshed best-effort, and the view+body is rendered.
///
/// **Sections decide what is fetched, not just what is printed.** `--without body` skips the
/// `GET /content` round-trip entirely, which is the whole reason the old `--meta-only` was
/// cheap; it is the same saving under a name that composes. `--without open-meta` is a
/// render-time drop rather than a saving, because `GET /api/resources/{id}` carries both tiers
/// unconditionally — one call either way, so there is nothing to skip.
pub fn show(config: &Config, params: ShowParams<'_>) -> Result<()> {
    let id = temper_workflow::operations::parse_ref(params.r#ref)?;

    let sections = resource_sections::resolve_sections(
        params.with,
        params.without,
        resource_sections::show_defaults(),
    )?;
    let want_body = sections.contains(ResourceSection::Body);

    let config_clone = config.clone();
    let (mut metadata, body) = crate::actions::runtime::with_client(move |client| {
        Box::pin(async move {
            // `get` returns a `ResourceView` — the same shape a `list` row is, with the
            // `open-meta` section filled. The tiers are what make a body-less `show` a
            // strict subset of the full one.
            let detail = client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;

            let body = if want_body {
                let resp = client
                    .resources()
                    .content(uuid::Uuid::from(id))
                    .await
                    .map_err(crate::actions::runtime::client_err_to_temper)?;

                // Per-resource projection refresh — best-effort, and only on the path that
                // actually holds a body. The projection file IS the body plus frontmatter,
                // so refreshing it from a body-less read would write a truncated file over a
                // complete one: a cheap read must not damage the cache.
                if let Err(e) = crate::projection::write_resource_file_from_parts(
                    &config_clone.vault_root,
                    &detail,
                    &resp,
                ) {
                    crate::output::warning(format!("could not refresh projection file: {e}"));
                }
                Some(resp.markdown)
            } else {
                None
            };

            let metadata = serde_json::to_value(&detail)
                .map_err(|e| TemperError::Api(format!("metadata serialize: {e}")))?;
            Ok((metadata, body))
        })
    })?;

    inject_ref(&mut metadata);
    if !sections.contains(ResourceSection::OpenMeta) {
        if let Some(obj) = metadata.as_object_mut() {
            obj.remove("open_meta");
        }
    }

    // Fetch every requested section BEFORE rendering: the JSON arm folds them into
    // one document, so nothing may be printed until all of them are in hand.
    //
    // `--edges` is the short spelling of `--with edges`; they are one request, not two, so
    // they OR rather than each triggering a fetch.
    let edges = if params.edges || sections.contains(ResourceSection::Edges) {
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
            let doc = build_show_document(metadata, body.as_deref(), edges, lineage, provenance)?;
            let filtered =
                temper_core::projection::apply_top_level_filter(doc, params.fields, "id")
                    .map_err(map_projection_error)?;
            let rendered = crate::format::render(&filtered, params.format)?;
            crate::output::plain(rendered);
        }
        // Toon is the human TTY surface: keep the frontmatter+body document, then append
        // each requested section as its own block. The one-document contract is a JSON
        // (agent-surface) invariant, not a Toon one.
        crate::format::OutputFormat::Toon => {
            let metadata =
                temper_core::projection::apply_top_level_filter(metadata, params.fields, "id")
                    .map_err(map_projection_error)?;
            let rendered = match body.as_deref() {
                Some(body) => crate::format::render_resource_show(&metadata, body, params.format)?,
                None => crate::format::render(&metadata, params.format)?,
            };
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
    /// Raw `--open-meta-add` JSON object string: the ADD channel's generic door, for
    /// list-valued open-tier keys the eight repeatable flags above do not name. Unioned
    /// with those flags into one patch by `build_open_meta_add_for_update`.
    pub open_meta_add: Option<&'a str>,
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

/// Parse an open-tier JSON object flag value (`--open-meta`, `--open-meta-add`).
///
/// The open tier is a key/value map, so the value MUST be a JSON object; a
/// malformed string, or a JSON array/scalar, is a hard error rather than a
/// silent drop (parse-don't-validate / escalate). Returns the object `Value`.
///
/// `flag` names the caller's flag in both messages. The two channels differ in
/// what they DO with the object, never in what shape they accept, so they share
/// one parser rather than two that would drift.
fn parse_open_meta_flag(flag: &str, raw: &str) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| TemperError::BadRequest(format!("{flag} must be valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(TemperError::BadRequest(format!(
            "{flag} must be a JSON object (e.g. '{{\"marker\":\"x\"}}')"
        )));
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

/// The update surface's REPLACE channel: the explicit `--open-meta` JSON object.
///
/// The repeatable list flags no longer feed this. They used to, and because the wire's
/// `open_meta` merges at the key level, `--tags docs` on a resource holding six tags wrote
/// a one-element list and destroyed the other five — silently, with a success response,
/// under a flag whose help read *"Add tag"*. They now travel on `open_meta_add`
/// ([`build_open_meta_add_for_update`]), which unions.
///
/// `--open-meta` keeps its exact prior meaning, including `{"tags":[]}` to clear a list —
/// the only way to clear one, and the reason the additive channel is a separate field
/// rather than a global "lists always union" rule that would have taken clearing away.
fn build_open_meta_for_update(params: &UpdateParams<'_>) -> Result<Option<serde_json::Value>> {
    let Some(raw) = params.open_meta else {
        return Ok(None);
    };
    let parsed = parse_open_meta_flag("--open-meta", raw)?;
    // Shape hard-error + discouraged-key warning, send-side; the server re-enforces it.
    validate_open_meta_send_side(&parsed)?;
    Ok(Some(parsed))
}

/// The update surface's ADD channel: the repeatable list flags (`--tags`/`--relates-to`/…)
/// AND the generic `--open-meta-add` door beside them.
///
/// The repeatable flags cover eight keys and no more, so before `--open-meta-add` existed
/// any other list-valued key — `reinforced` is the case that first needed one — was
/// reachable from the CLI only through `--open-meta`, which REPLACES: writing one date
/// destroyed every date already stored, silently, with a success response. MCP's
/// `update_resource` has carried a generic `open_meta_add` throughout
/// (`temper-mcp/src/tools/resources.rs:275`) and the field exists end to end on the wire,
/// so this closes a door that was missing rather than adding a capability.
///
/// One generic flag rather than a `--reinforced` list flag per convention: a per-key flag
/// solves one key and leaves the next additive convention to have this conversation again.
///
/// Both inputs feed the single `open_meta_add` wire field, so where they name the same key
/// the two sets UNION — the channel's own semantics applied to its own two doors, and the
/// server unions over the stored value on top of that. Validation runs once, on the merged
/// object, so a discouraged key named by both warns once rather than twice.
///
/// Returns `None` when neither door was used, so a frontmatter-only update PATCHes nothing
/// on the open tier.
fn build_open_meta_add_for_update(params: &UpdateParams<'_>) -> Result<Option<serde_json::Value>> {
    let named = build_partial_open_meta_from_args(params);
    let generic = params
        .open_meta_add
        .map(|raw| parse_open_meta_flag("--open-meta-add", raw))
        .transpose()?;
    let Some(merged) = union_open_meta_patches(named, generic) else {
        return Ok(None);
    };
    validate_open_meta_send_side(&merged)?;
    Ok(Some(merged))
}

/// Union two open-tier patches bound for the one `open_meta_add` wire field.
///
/// Both arguments are objects (or absent). Where a key is a list on both sides the lists
/// concatenate, skipping values already present — the same collapse the server's
/// `union_list` applies, done here only so the patch that leaves the CLI is already the
/// set the caller asked for. Any other collision takes the `--open-meta-add` value, which
/// is then the server's to judge: a non-list on the add channel is refused with a 400
/// rather than replacing the stored list, and that refusal is deliberately left to the one
/// place that owns the rule.
fn union_open_meta_patches(
    named: Option<serde_json::Value>,
    generic: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let (named, generic) = match (named, generic) {
        (None, None) => return None,
        (Some(only), None) | (None, Some(only)) => return Some(only),
        (Some(n), Some(g)) => (n, g),
    };
    let mut out = named.as_object().cloned().unwrap_or_default();
    for (key, incoming) in generic.as_object().cloned().unwrap_or_default() {
        match (out.get_mut(&key), &incoming) {
            (Some(serde_json::Value::Array(existing)), serde_json::Value::Array(add)) => {
                for item in add {
                    if !existing.contains(item) {
                        existing.push(item.clone());
                    }
                }
            }
            _ => {
                out.insert(key, incoming);
            }
        }
    }
    Some(serde_json::Value::Object(out))
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

/// Resolve the update target: parse the ref to an id, read the current server row
/// (context-free) for its doctype + home context, and validate any `--type-to` target
/// before the command is built. Returns the `(id, row)` pair the rest of `update` threads on.
///
/// **The resource's OWN doctype is deliberately not gated here.** It used to be, and the
/// gate refused resources the system itself creates: doc type is a free-text
/// `kb_properties` row with no lookup table and no server-side parse (`DocType::from_str`
/// appears nowhere in temper-api / temper-services / temper-substrate / temper-mcp), and
/// `resource create` discards its own parse error, so the vocabulary is open in every
/// direction but this one. Production carries 22 live `kernel_landmark` and 4 live
/// `cogmap_charter` resources `[observed — 2026-08-02]`, and `update` refused all 26 with
/// *"unknown doctype … expected one of …"* — a message wrong about the system it describes.
///
/// The gate was also inert: its parse result was discarded. The remaining doctype check is
/// [`validate_update_args`], which takes the name as a **string** and reaches
/// `schema::updatable_fields` — which re-parses it — only when a scalar flag (`--stage`,
/// `--mode`, …) is actually set. So an out-of-vocabulary resource now accepts
/// `--tags`/`--open-meta-add`/`--body`, and still refuses `--stage`. The refusal is
/// **scoped, not renamed**: it is the same "unknown doctype" message, now raised only where
/// a doc-type schema is genuinely required rather than on every update of the resource.
///
/// `--type-to` keeps its gate: choosing a conversion TARGET is a caller's assertion about
/// what the resource should become, not an existing fact to be re-litigated.
fn resolve_update_target(
    params: &UpdateParams<'_>,
) -> Result<(
    temper_core::types::ids::ResourceId,
    temper_core::types::resource_view::ResourceView,
)> {
    let id = temper_workflow::operations::parse_ref(params.r#ref)?;
    // Update needs only the row (home context + doctype for schema-keyed flag validation);
    // `get` also carries both meta tiers.
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .get(uuid::Uuid::from(id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;
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
    //    and home context), validating any `--type-to` target.
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
        open_meta_add: build_open_meta_add_for_update(params)?,
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
    let updated_row: temper_core::types::resource_view::ResourceView = output.value;

    // 6. Projection refresh: rewrite the affected projection file from
    //    the returned server row. Best-effort — a projection write
    //    failure must not fail the update.
    if let Err(e) = runtime.block_on(crate::projection::write_resource_file(
        &client,
        &config.vault_root,
        &updated_row,
    )) {
        output::warning(format!("could not rewrite projection file: {e}"));
    }

    // 7. Emit the flat UpdateActionResult to stdout (Task 9: replaces the
    //    bespoke { "temper-slug", "content_hash" } shape).
    let result = UpdateActionResult {
        status: "ok",
        resource: updated_row,
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
    })?;

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
    // Transitional narrowing — see the note in `create`.
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

    /// Every value the goal schema declares is accepted by the `--status` filter.
    ///
    /// Enumerated rather than spot-checked: the filter and the write path must accept the
    /// same set, and a value writable but not filterable would be a status you can set and
    /// then never find.
    #[test]
    fn status_filter_accepts_every_schema_status() {
        for status in ["active", "completed", "paused", "cancelled"] {
            assert!(
                validate_status_filter(status).is_ok(),
                "--status {status} is declared by goal.schema.json and must be accepted"
            );
        }
    }

    /// The guard for the original defect: `--status bogus-value` returned all 43 goals.
    ///
    /// Paired deliberately with the accept-test above. A filter test alone cannot see
    /// "accepts everything" — that was green for every valid value while the bug was live.
    #[test]
    fn status_filter_rejects_a_value_outside_the_enum() {
        let err = validate_status_filter("bogus-value")
            .expect_err("a status outside the goal enum must be rejected, not scanned");
        let msg = err.to_string();
        assert!(
            msg.contains("bogus-value"),
            "the error must name the offending value so the caller can fix it; got: {msg}"
        );
    }

    /// A type-scoped filter on the wrong doc type errors instead of claiming to be ignored.
    ///
    /// All three of `--stage`/`--goal`/`--status` are sent to the server unconditionally,
    /// so the previous hint — "ignored for {type}" — was false, and false in the expensive
    /// direction: `--type goal --stage backlog` printed it above `total: 0` while 48 goals
    /// existed, inviting the reader to conclude no goals matched.
    #[test]
    fn mismatched_filter_error_names_the_flag_and_both_types() {
        let err = mismatched_filter_err("--stage", "task", "goal");
        let msg = err.to_string();
        assert!(msg.contains("--stage"), "must name the flag; got: {msg}");
        assert!(
            msg.contains("task") && msg.contains("goal"),
            "must name both the type it applies to and the type given; got: {msg}"
        );
        assert!(
            !msg.contains("ignored"),
            "must not repeat the word that made the old hint false; got: {msg}"
        );
    }

    /// No `--tag` is no filter — `None`, never `Some("")`. An empty CSV would reach the
    /// server as a filter that matches everything, which is the same "flag present, nothing
    /// filtered" shape this filter's goal exists to eliminate.
    #[test]
    fn no_tag_flag_sends_no_tag_filter() {
        assert_eq!(resolve_tag_csv(&[]).unwrap(), None);
    }

    /// Repeated `--tag` joins to the CSV the GET carries. Order is preserved here; the server
    /// sorts and dedupes, so this only has to be lossless.
    #[test]
    fn repeated_tags_join_to_csv() {
        let tags = vec!["ci".to_string(), "security".to_string()];
        assert_eq!(
            resolve_tag_csv(&tags).unwrap().as_deref(),
            Some("ci,security")
        );
    }

    /// A tag containing a comma is REFUSED, not split. Over the wire `--tag "a,b"` would
    /// arrive as two tags and — because multiple tags AND — return a strictly narrower set
    /// than the caller asked for, silently. Escalate instead of quietly answering a
    /// different question.
    #[test]
    fn a_tag_containing_a_comma_is_refused_not_split() {
        let tags = vec!["a,b".to_string()];
        let err = resolve_tag_csv(&tags)
            .expect_err("a comma in a tag must be refused — the transport would split it");
        let msg = err.to_string();
        assert!(
            msg.contains("comma"),
            "the refusal must name the comma so the caller can fix it; got: {msg}"
        );
    }

    /// An empty or whitespace-only `--tag` matches nothing and is almost certainly a shell
    /// accident (`--tag "$UNSET"`). Refuse rather than return a confident empty page.
    #[test]
    fn an_empty_tag_is_refused() {
        assert!(resolve_tag_csv(&["".to_string()]).is_err());
        assert!(resolve_tag_csv(&["   ".to_string()]).is_err());
    }

    /// A `ListParams` with every field defaulted to "absent", for tests that vary one axis.
    /// Borrows the slices the caller owns, because `ListParams` holds references.
    fn list_params<'a>(tag: &'a [String], cogmap: &'a [String]) -> ListParams<'a> {
        ListParams {
            doc_type: None,
            tag,
            context: None,
            cogmap,
            limit: None,
            all: false,
            offset: None,
            page: None,
            sort: None,
            title_contains: None,
            stage: None,
            goal: None,
            status: None,
            format: crate::format::OutputFormat::Json,
            with: &[],
            without: &[],
            fields: &[],
        }
    }

    /// The empty set — a default `list`.
    fn no_sections() -> SectionSet {
        SectionSet::default()
    }

    /// What `--with open-meta` resolves to.
    fn open_meta_section() -> SectionSet {
        [ResourceSection::OpenMeta].into_iter().collect()
    }

    /// Every `--flag` on `ListParams` lands in the wire field it names.
    ///
    /// This is the guard the CLI list path never had. `list` returns `Result<()>` and prints
    /// its rows, so no test downstream of `list_api_params` can observe an assignment: the one
    /// e2e that drives `commands::resource::list` passed every filter as absent, which means
    /// deleting `tags: resolve_tag_csv(...)` — or `stage`, `goal`, `q`, `sort`, `limit`,
    /// `offset` — kept the whole suite green. The tag filter's own e2e tests do not close this:
    /// they call `client.resources().list(&ResourceListParams { tags: ... })` directly, one
    /// call BELOW the seam, so they prove the CSV filters and prove nothing about how it got
    /// there.
    ///
    /// Asserting field-by-field rather than against a whole struct is deliberate: a single
    /// `assert_eq!` on `ResourceListParams` would fail as one opaque diff, and would need
    /// rewriting every time an unrelated field is added.
    #[test]
    fn every_list_flag_reaches_its_api_field() {
        use temper_workflow::types::resource::{ResourceSortField, SortOrder};

        let tags = vec!["ci".to_string(), "security".to_string()];
        let goal_uuid = uuid::Uuid::now_v7();
        let goal_ref = format!("some-goal-{goal_uuid}");

        let mut params = list_params(&tags, &[]);
        params.doc_type = Some("task");
        params.context = Some("@me/temper");
        params.stage = Some("in-progress");
        params.goal = Some(goal_ref.as_str());
        params.title_contains = Some("filter");
        params.sort = Some("created:asc");
        params.limit = Some(7);
        params.offset = Some(3);

        let api =
            list_api_params(&params, &no_sections()).expect("a well-formed ListParams must build");

        assert_eq!(api.doc_type_name.as_deref(), Some("task"), "--type");
        assert_eq!(api.tags.as_deref(), Some("ci,security"), "--tag");
        assert_eq!(api.context_ref.as_deref(), Some("@me/temper"), "--context");
        assert_eq!(api.stage.as_deref(), Some("in-progress"), "--stage");
        assert_eq!(api.goal, Some(goal_uuid), "--goal (resolved from the ref)");
        assert_eq!(api.q.as_deref(), Some("filter"), "--title-contains -> q");
        assert_eq!(api.limit, Some(7), "--limit");
        assert_eq!(api.offset, Some(3), "--offset");
        // The sort enums derive no `PartialEq`, so these match rather than compare. Both are
        // closed unit-variant sets, so a `matches!` here is exhaustive over what can be sent.
        assert!(
            matches!(api.sort, Some(ResourceSortField::Created)),
            "--sort field must reach `sort`; got {:?}",
            api.sort
        );
        assert!(
            matches!(api.order, Some(SortOrder::Asc)),
            "--sort direction must reach `order`; got {:?}",
            api.order
        );
    }

    /// `--status` and `--cogmap` cannot ride the fixture above — `--status` applies only to
    /// `--type goal`, and `--cogmap` is mutually exclusive with `--context`. They reach their
    /// fields on the same terms, and are asserted here so the seam covers all of them.
    #[test]
    fn status_and_cogmap_reach_their_api_fields() {
        let cogmap_uuid = uuid::Uuid::now_v7();
        let cogmaps = vec![format!("some-map-{cogmap_uuid}")];

        let mut goal_params = list_params(&[], &[]);
        goal_params.doc_type = Some("goal");
        goal_params.status = Some("active");
        let api = list_api_params(&goal_params, &no_sections())
            .expect("a goal-scoped --status must build");
        assert_eq!(api.status.as_deref(), Some("active"), "--status");

        let cogmap_params = list_params(&[], &cogmaps);
        let api = list_api_params(&cogmap_params, &no_sections())
            .expect("a --cogmap with no --context must build");
        assert_eq!(
            api.cogmap_ids.as_deref(),
            Some(cogmap_uuid.to_string().as_str()),
            "--cogmap (resolved from the ref)"
        );
    }

    /// Asking for a section changes the `sections` param and **nothing else** — the page
    /// size included.
    ///
    /// This is what makes one builder safe to share. `list` and `list --meta-only` carried
    /// character-identical copies of the param build until they were collapsed; the risk of
    /// collapsing them is that a difference gets flattened away, so the one surviving
    /// difference is pinned. `sections` must stay `None` on the default path rather than
    /// `Some("")` — the field is `skip_serializing_if = "Option::is_none"`, so the two are
    /// different wires, which is why `SectionSet::to_csv` answers `None` for the empty set.
    ///
    /// **`limit` is now inside the equality**, not excluded from it. It used to be the
    /// second permitted difference (`DEFAULT_META_LIST_LIMIT = 50`); with one default,
    /// asking for a section that no longer changes the row type must not change how many
    /// rows come back either.
    #[test]
    fn asking_for_a_section_changes_only_the_sections_param() {
        let params = list_params(&[], &[]);

        let full = list_api_params(&params, &no_sections()).expect("full list builds");
        let meta = list_api_params(&params, &open_meta_section()).expect("meta list builds");

        assert_eq!(full.limit, Some(DEFAULT_LIST_LIMIT as i64));
        assert_eq!(
            meta.limit, full.limit,
            "one default: a section request must not resize the page"
        );
        assert_eq!(full.sections, None, "the default path asks for no sections");
        assert_eq!(
            meta.sections.as_deref(),
            Some("open-meta"),
            "the section path asks for a PART of the one shape, not a second response type"
        );

        // Every OTHER field agrees, so the shared builder is not flattening a second
        // difference. Compared through serde because `ResourceListParams` derives no
        // `PartialEq` — and comparing the wire form is the stronger check anyway, since the
        // wire is what the two paths actually differ on.
        let mut full_wire = serde_json::to_value(&full).expect("serialize full params");
        let mut meta_wire = serde_json::to_value(&meta).expect("serialize meta params");
        for wire in [&mut full_wire, &mut meta_wire] {
            let obj = wire.as_object_mut().expect("params serialize to an object");
            obj.remove("sections");
        }
        assert_eq!(
            full_wire, meta_wire,
            "the two paths must differ ONLY in sections"
        );
    }

    /// `--all` overrides the default page cap on both paths — it is the one flag whose effect
    /// is an ABSENT wire value, so an assignment test cannot see it and it needs its own case.
    #[test]
    fn all_removes_the_page_cap_on_both_paths() {
        let mut params = list_params(&[], &[]);
        params.all = true;

        assert_eq!(
            list_api_params(&params, &no_sections())
                .expect("builds")
                .limit,
            None
        );
        assert_eq!(
            list_api_params(&params, &open_meta_section())
                .expect("builds")
                .limit,
            None
        );
    }

    /// `--stage` on a mismatched `--type` still errors, but omitting `--type` is NOT a
    /// mismatch. This is the guard that changed when `--type` became optional: with no type
    /// named there is no conflicting claim to refuse, and `--stage backlog` alone honestly
    /// narrows to backlog tasks (only tasks carry a stage).
    #[test]
    fn a_type_scoped_filter_errors_only_when_a_type_was_named() {
        // Named and mismatched -> still an error. This is the behavior `--type`'s becoming
        // optional must not have weakened.
        assert!(check_type_scoped_filters(Some("goal"), Some("backlog"), None, None).is_err());
        assert!(check_type_scoped_filters(Some("task"), None, None, Some("active")).is_err());
        assert!(check_type_scoped_filters(Some("goal"), None, Some("ref"), None).is_err());

        // Named and matching -> no refusal.
        assert!(check_type_scoped_filters(Some("task"), Some("backlog"), None, None).is_ok());
        assert!(check_type_scoped_filters(Some("goal"), None, None, Some("active")).is_ok());

        // Type OMITTED -> no refusal for any of them. There is no stated type to conflict with.
        assert!(check_type_scoped_filters(None, Some("backlog"), None, None).is_ok());
        assert!(check_type_scoped_filters(None, None, None, Some("active")).is_ok());
        assert!(check_type_scoped_filters(None, None, Some("ref"), None).is_ok());
    }

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
        assert_eq!(resolve_list_limit(true, None), None);
        // `--all` wins over any (clap-excluded) limit.
        assert_eq!(resolve_list_limit(true, Some(5)), None);
    }

    /// An explicit `--limit` is honoured unchanged — the default applies only when the
    /// caller asked for nothing, and no clamp exists on either side.
    #[test]
    fn resolve_list_limit_uses_explicit_then_the_one_default() {
        assert_eq!(resolve_list_limit(false, Some(5)), Some(5));
        assert_eq!(resolve_list_limit(false, Some(10_000)), Some(10_000));
        assert_eq!(
            resolve_list_limit(false, None),
            Some(DEFAULT_LIST_LIMIT as i64)
        );
    }

    /// `--page 1` is the first page, which is offset 0 — not offset 20.
    #[test]
    fn page_one_is_offset_zero() {
        assert_eq!(resolve_list_offset(Some(1), None, None), 0);
        assert_eq!(resolve_list_offset(Some(1), None, Some(5)), 0);
    }

    /// **`--page` counts in the effective page size, not in a hardcoded default.**
    ///
    /// `--page 3 --limit 5` starts at row 10. Resolving against a hardcoded 20 would give
    /// 40 — a page that skips 30 rows and looks entirely plausible from the outside, which
    /// is what makes this the failure mode worth a named witness rather than a comment.
    #[test]
    fn page_resolves_against_the_effective_limit() {
        assert_eq!(resolve_list_offset(Some(3), None, Some(5)), 10);
        // And with no `--limit`, the same page number counts in the one default.
        assert_eq!(
            resolve_list_offset(Some(3), None, None),
            2 * DEFAULT_LIST_LIMIT
        );
    }

    /// With no `--page`, `--offset` passes through verbatim; with neither, the walk starts
    /// at the top.
    #[test]
    fn offset_passes_through_when_no_page_is_given() {
        assert_eq!(resolve_list_offset(None, Some(37), Some(5)), 37);
        assert_eq!(resolve_list_offset(None, None, Some(5)), 0);
    }

    /// **`truncated` is read off the wire, never recomputed from the rendered page.**
    ///
    /// The response here is deliberately self-contradictory by the client's OLD rule:
    /// `offset + returned == total`, which `inject_truncation_signal` would have rendered as
    /// `truncated: false`. The server said `true`. The envelope must say what the server
    /// said — the client is not a second opinion about a page it did not build.
    #[test]
    fn truncated_comes_from_the_wire_not_the_client() {
        use temper_workflow::types::resource::{ResourceFacets, ResourceListResponse};

        let response = ResourceListResponse {
            rows: Vec::new(),
            total: 5,
            facets: ResourceFacets::default(),
            returned: 5,
            // `offset + returned == total`: the retired client-side derivation says `false`.
            truncated: true,
            limit: Some(5),
            offset: 0,
        };

        let envelope = build_list_envelope(&response, &[]).expect("envelope builds");

        assert_eq!(
            envelope["truncated"],
            serde_json::json!(true),
            "the wire's verdict survives: {envelope}"
        );
        assert_eq!(
            envelope["returned"],
            serde_json::json!(5),
            "`returned` is the wire field, not `rows.len()` — the page here carries no rows"
        );
        // And the rest of the paging state rides along, which is what a caller pages on.
        assert_eq!(envelope["limit"], serde_json::json!(5));
        assert_eq!(envelope["offset"], serde_json::json!(0));
        assert_eq!(envelope["total"], serde_json::json!(5));
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
            open_meta_add: None,
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

    /// A doc type the Rust enum does not name is still updatable, as long as the flags in
    /// play need no doc-type schema.
    ///
    /// Doc type is a free-text `kb_properties` row: no lookup table, no server-side parse,
    /// and `resource create` discards its own. The system therefore mints types the enum
    /// never learned — 22 live `kernel_landmark` and 4 live `cogmap_charter` in production
    /// `[observed — 2026-08-02]` — and `resolve_update_target` used to refuse every one of
    /// them before reading a single flag.
    ///
    /// `kernel_landmark` is used verbatim rather than a made-up string precisely because it
    /// is a real type the substrate writes; a fictional one would pin the mechanism while
    /// leaving the case that motivated it untested.
    #[test]
    fn an_out_of_vocabulary_doctype_is_updatable_by_flags_needing_no_schema() {
        let mut params = empty_update_params("foo");
        params.open_meta = Some(r#"{"marker":"x"}"#);
        let tags = vec!["x".to_string()];
        params.tags = &tags;

        assert!(
            validate_update_args(&params, "kernel_landmark").is_ok(),
            "open-tier flags need no doc-type schema, so an unknown doctype must not \
             block them — 26 live production resources are exactly this case"
        );
    }

    /// The converse, and the reason removing the blanket gate is safe: a flag that DOES
    /// need the doc-type schema still refuses.
    ///
    /// The refusal is scoped, not renamed — `schema::updatable_fields` re-parses the type,
    /// so it is the same "unknown doctype" message, now raised only where a schema is
    /// genuinely required instead of on every update of the resource.
    #[test]
    fn a_schema_keyed_flag_still_refuses_an_out_of_vocabulary_doctype() {
        let mut params = empty_update_params("foo");
        params.stage = Some("done");

        assert!(
            validate_update_args(&params, "kernel_landmark").is_err(),
            "--stage is validated against the doc-type schema, so a type with no schema \
             must still be refused; dropping the blanket gate must not drop this"
        );
    }

    /// A known doctype is unaffected in both directions — the change is about types the
    /// enum does not name, and a guard that also loosened the ordinary path would be a
    /// different change wearing this one's clothes.
    #[test]
    fn a_known_doctype_still_validates_its_scalar_flags() {
        let mut params = empty_update_params("foo");
        params.stage = Some("done");
        assert!(validate_update_args(&params, "task").is_ok());

        let mut bad = empty_update_params("foo");
        bad.stage = Some("done");
        assert!(
            validate_update_args(&bad, "memory").is_err(),
            "a memory has no temper-stage field, so --stage on one is still a schema error"
        );
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
        let v = parse_open_meta_flag("--open-meta", r#"{"marker":"x","n":1}"#).expect("object");
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
        assert!(parse_open_meta_flag("--open-meta", r#"["a","b"]"#).is_err());
        assert!(parse_open_meta_flag("--open-meta", "42").is_err());
        // Malformed JSON → hard error (never a silent drop).
        assert!(parse_open_meta_flag("--open-meta", "{not json").is_err());
    }

    /// The refusal names the flag the caller actually typed.
    ///
    /// Both channels share one parser, so the flag name is a parameter rather than a
    /// literal — and a shared parser that hardcodes one caller's flag sends everyone who
    /// mistypes `--open-meta-add` to look at `--open-meta`.
    #[test]
    fn a_malformed_open_tier_flag_is_refused_under_its_own_name() {
        let err = parse_open_meta_flag("--open-meta-add", "{not json").expect_err("malformed");
        let msg = err.to_string();
        assert!(
            msg.contains("--open-meta-add"),
            "the refusal must name --open-meta-add, not the flag it shares a parser with. \
             Got: {msg}"
        );
        let err = parse_open_meta_flag("--open-meta-add", "42").expect_err("scalar");
        assert!(err.to_string().contains("--open-meta-add"));
    }

    /// The two open-tier channels stay separate all the way to the wire.
    ///
    /// This replaces `build_open_meta_for_update_merges_explicit_over_list_flags`, which
    /// asserted the merge that WAS the data-loss bug: folding `--tags` into `open_meta`
    /// made it a key-level replace, so adding one tag destroyed the rest. The test is
    /// rewritten rather than deleted because the merge it pinned is precisely what must
    /// never come back — a deleted test leaves nothing standing between here and it.
    #[test]
    fn list_flags_and_explicit_open_meta_travel_on_separate_channels() {
        let mut params = empty_update_params("foo");
        let tags = vec!["a".to_string(), "b".to_string()];
        params.tags = &tags;
        params.open_meta = Some(r#"{"marker":"x"}"#);

        let replace = build_open_meta_for_update(&params)
            .expect("ok")
            .expect("some open_meta");
        assert_eq!(replace.get("marker"), Some(&serde_json::json!("x")));
        assert!(
            replace.get("tags").is_none(),
            "--tags must NOT ride the replace channel; that is what destroyed sibling \
             tags. Got: {replace}"
        );

        let add = build_open_meta_add_for_update(&params)
            .expect("ok")
            .expect("some open_meta_add");
        assert_eq!(add.get("tags"), Some(&serde_json::json!(["a", "b"])));
        assert!(
            add.get("marker").is_none(),
            "--open-meta must NOT ride the add channel; it is the only way to replace \
             or clear a list. Got: {add}"
        );
    }

    /// The add channel is absent when neither door was used, so a frontmatter-only
    /// update PATCHes nothing on the open tier.
    #[test]
    fn open_meta_add_is_none_without_list_flags() {
        let mut params = empty_update_params("foo");
        params.open_meta = Some(r#"{"marker":"x"}"#);
        assert!(build_open_meta_add_for_update(&params)
            .expect("ok")
            .is_none());
    }

    /// A key none of the eight repeatable flags names still reaches the ADD channel,
    /// and reaches ONLY it.
    ///
    /// This is the whole point of the generic door. `reinforced` — a memory's record of
    /// the days it proved load-bearing — is exactly such a key, and before this flag the
    /// only CLI route to it was `--open-meta`, which replaces: one date written over
    /// however many were stored, silently, with a success response. The two halves are
    /// asserted together here because the defect is not "the add channel is empty", it is
    /// "the value went down the destroying one".
    ///
    /// What this test can and cannot see: it pins the ROUTING, which is the half the CLI
    /// owns. That `open_meta_add` then unions server-side rather than replacing is pinned
    /// against a real database by `open_meta_add_unions_instead_of_replacing`
    /// (`temper-services/tests/open_meta_roundtrip_test.rs`).
    #[test]
    fn an_unnamed_key_reaches_the_add_channel_and_never_the_replace_one() {
        let mut params = empty_update_params("foo");
        params.open_meta_add = Some(r#"{"reinforced":["2026-08-02"]}"#);

        let add = build_open_meta_add_for_update(&params)
            .expect("ok")
            .expect("some open_meta_add");
        assert_eq!(
            add.get("reinforced"),
            Some(&serde_json::json!(["2026-08-02"]))
        );

        assert!(
            build_open_meta_for_update(&params).expect("ok").is_none(),
            "--open-meta-add must contribute NOTHING to the replace channel; anything it \
             puts there would overwrite the stored list it exists to preserve"
        );
    }

    /// Where both add-channel doors name one key, the two sets union rather than one
    /// winning — they are two doors onto a single wire field, so a caller who used both
    /// asked for both.
    #[test]
    fn the_two_add_doors_union_on_a_shared_key() {
        let mut params = empty_update_params("foo");
        let tags = vec!["a".to_string(), "b".to_string()];
        params.tags = &tags;
        params.open_meta_add = Some(r#"{"tags":["b","c"],"reinforced":["2026-08-02"]}"#);

        let add = build_open_meta_add_for_update(&params)
            .expect("ok")
            .expect("some open_meta_add");
        assert_eq!(
            add.get("tags"),
            Some(&serde_json::json!(["a", "b", "c"])),
            "a value present on both sides is carried once, in flag-then-generic order"
        );
        assert_eq!(
            add.get("reinforced"),
            Some(&serde_json::json!(["2026-08-02"])),
            "a key only the generic door names survives the union"
        );
    }

    /// The send-side shape gate covers the generic door too — it runs on the MERGED patch,
    /// so a key arriving that way is held to the same schema as one arriving by flag.
    #[test]
    fn build_open_meta_add_rejects_a_misshaped_recognized_key() {
        let mut params = empty_update_params("foo");
        params.open_meta_add = Some(r#"{"tags":[1,2]}"#);
        assert!(build_open_meta_add_for_update(&params).is_err());
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
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::resource_view::ResourceView;

    use super::{
        render_action_result_with_ref, CreateActionResult, DeleteActionResult, UpdateActionResult,
    };

    /// Build a minimal `ResourceView` fixture for action result tests.
    pub(super) fn make_resource_row(
        _slug: &str,
        doc_type: &str,
        title: &str,
        context: &str,
    ) -> ResourceView {
        ResourceView {
            id: ResourceId(uuid::Uuid::nil()),
            r#ref: String::new(),
            title: title.to_string(),
            origin_uri: "test://origin".to_string(),
            kb_context_id: Some(ContextId(uuid::Uuid::nil())),
            context_name: Some(context.to_string()),
            context_slug: Some(context.to_string()),
            context_owner_ref: Some("@me".to_string()),
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: doc_type.to_string(),
            owner_handle: "@me".to_string(),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            body_hash: None,
            ingest_state: Some(temper_core::types::resource::IngestState::Complete),
            body_storage: Some(temper_core::types::resource::BodyStorage::Derived),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            content: None,
        }
        .with_derived_refs()
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
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::resource_view::ResourceView;

    /// Task 7: verify that `render()` passthrough includes internal wire fields
    /// like `body_hash` that the old `row_to_frontmatter_value` + `render_server_rows`
    /// path deliberately dropped. This is the canary for the breaking change.
    #[test]
    fn render_resource_list_json_passes_wire_type_with_internals() {
        let rows: Vec<ResourceView> = vec![ResourceView {
            id: ResourceId(uuid::Uuid::nil()),
            r#ref: String::new(),
            title: "Test Resource".to_string(),
            origin_uri: "test://origin".to_string(),
            kb_context_id: Some(ContextId(uuid::Uuid::nil())),
            context_name: Some("temper".to_string()),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "research".to_string(),
            owner_handle: "@me".to_string(),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            body_hash: Some("abc123deadbeef".to_string()),
            ingest_state: Some(temper_core::types::resource::IngestState::Complete),
            body_storage: Some(temper_core::types::resource::BodyStorage::Derived),
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            content: None,
        }
        .with_derived_refs()];

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
mod list_section_projection_tests {
    use temper_core::projection::apply_top_level_filter;

    #[test]
    fn list_meta_filter_applies_per_row_and_preserves_envelope() {
        // Build a stub meta-list envelope. Rows are `ResourceView`-shaped (identity plus
        // both tiers), so they carry `title`/`doc_type_name` too.
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

    /// A titleless row gets no `ref` rather than a fabricated `-<uuid>` one. Surfaced by the
    /// #330 differential e2e: the malformed ref made the then-`--meta-only` projection
    /// disagree with the full `show` on the same key.
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
mod show_projection_tests {
    use temper_core::projection::apply_top_level_filter;
    use temper_core::types::managed_meta::ManagedMeta;
    use temper_core::types::resource_view::ResourceView;

    fn fake_meta_response() -> ResourceView {
        ResourceView {
            managed_meta: ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..Default::default()
            },
            open_meta: Some(serde_json::json!({"tags": ["x"]})),
            ..super::action_result_tests::make_resource_row("s", "task", "A Task", "temper")
        }
    }

    #[test]
    fn show_fields_filter_preserves_anchor_and_managed_meta_only() {
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
    fn show_no_fields_returns_full_response() {
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
            Some("# body\n"),
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
        let doc = build_show_document(metadata, Some("b"), None, None, None)
            .expect("build show document");

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
