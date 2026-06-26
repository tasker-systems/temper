//! `DbBackend` — the substrate backend behind the `Backend` trait (the single backend post-WS6-collapse).
//!
//! Reads delegate to `temper_substrate::readback`; writes compose `temper_substrate::writes`. The SQL is
//! unqualified against the one schema (`public`); the connection's search_path resolves all references.
//!
//! The full-row read (`show_resource`) maps the substrate readback (`readback::resource_row`) to the
//! native `ResourceRow` — real timestamps (event-sourced from `kb_events.occurred_at`), name-only
//! doc type, no fabricated fields. The §7-dissolved fields (`kb_doc_type_id`, `slug`, `managed_hash`,
//! `open_hash`) are gone. See `native_resource_row` and the historical §9 parity floor.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::types::graph;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::reconcile::{ReconcileCogmapRequest, ReconcileOutcome};
use temper_workflow::operations::{
    AssertRelationship, Backend, CommandOutput, CreateResource, DeleteResource, FoldRelationship,
    ListResources, ReconcileCognitiveMap, ResourceSummary, RetypeRelationship,
    ReweightRelationship, SearchHit, SearchResources, ShowResource, Surface, UpdateResource,
};
use temper_workflow::types::resource::ResourceRow;

use temper_substrate::keys::{key_fate, KeyFate};
use temper_substrate::readback;
use temper_substrate::writes;

/// Bridge a temper-substrate (`anyhow`) error into `TemperError` without naming `anyhow` (temper-api does not
/// depend on it) — `anyhow::Error: Display`, so `to_string()` carries the message.
fn api_err(e: impl std::fmt::Display) -> TemperError {
    TemperError::Api(e.to_string())
}

/// Map a typed [`readback::ReadbackError`] to the surface status, splitting the two deny modes the
/// single-resource reads can return: not-visible is the leak-safe deny → **404** (`NotFound`), never 403
/// (403 confirms existence) and never 500 (it is not a system failure); a genuine fault stays **500**
/// (`Api`). Collapsing both into NotFound — the pre-typing behavior on every substrate single-read
/// surface — masked real faults as 404. Shared by `native_resource_row` and the substrate read's
/// `get_content`/`get_meta` arms so the mapping lives in exactly one place.
pub(crate) fn map_readback_err(e: readback::ReadbackError) -> TemperError {
    match e {
        readback::ReadbackError::NotVisible { resource_id, .. } => {
            TemperError::NotFound(format!("resource {resource_id} not found"))
        }
        readback::ReadbackError::Fault(inner) => TemperError::Api(inner.to_string()),
    }
}

/// graph::EdgeKind → temper-substrate's affinity::EdgeKind (identical 4-variant taxonomy — 1:1, no §4 remap).
fn map_edge_kind(k: graph::EdgeKind) -> temper_substrate::affinity::EdgeKind {
    use temper_substrate::affinity::EdgeKind as N;
    match k {
        graph::EdgeKind::Express => N::Express,
        graph::EdgeKind::Contains => N::Contains,
        graph::EdgeKind::LeadsTo => N::LeadsTo,
        graph::EdgeKind::Near => N::Near,
    }
}

/// graph::Polarity → temper-substrate's payloads::EdgePolarity.
fn map_polarity(p: graph::Polarity) -> temper_substrate::payloads::EdgePolarity {
    use temper_substrate::payloads::EdgePolarity as N;
    match p {
        graph::Polarity::Forward => N::Forward,
        graph::Polarity::Inverse => N::Inverse,
    }
}

/// Map a wire [`PackedChunk`](temper_core::types::ingest::PackedChunk) — the client's
/// extract→chunk→embed output — to the substrate-native `IncomingChunk` the no-embed block constructor
/// consumes. Field-for-field; the only widening is `u32`/`u8` → `i32`/`i16` (the substrate column types).
fn packed_to_incoming(
    c: &temper_core::types::ingest::PackedChunk,
) -> temper_substrate::content::IncomingChunk {
    temper_substrate::content::IncomingChunk {
        chunk_index: c.chunk_index as i32,
        content_hash: c.content_hash.clone(),
        content: c.content.clone(),
        embedding: c.embedding.clone(),
        header_path: c.header_path.clone(),
        heading_depth: c.heading_depth as i16,
    }
}

/// Unpack a caller-supplied `chunks_packed` blob into substrate-native `IncomingChunk`s, ordered by
/// `chunk_index` so the body-hash merkle matches the substrate's stored `body_hash`. A malformed blob is
/// the caller's fault → `BadRequest`.
fn unpack_incoming_chunks(
    packed: &str,
) -> Result<Vec<temper_substrate::content::IncomingChunk>, TemperError> {
    let mut chunks = temper_core::types::ingest::unpack_chunks(packed)
        .map_err(|e| TemperError::BadRequest(format!("invalid chunks_packed: {e}")))?;
    chunks.sort_by_key(|c| c.chunk_index);
    Ok(chunks.iter().map(packed_to_incoming).collect())
}

/// Strip a stray top-level `provenance` key from a clustering-facet object (Decision #6): `provenance`
/// is a per-key property the reconciler STAMPS on create, never a clustering facet. Returns the object
/// minus that key (cloned); a non-object value passes through unchanged.
fn strip_provenance_facet(facets: &serde_json::Value) -> serde_json::Value {
    match facets.as_object() {
        Some(obj) => {
            let mut out = obj.clone();
            out.remove("provenance");
            serde_json::Value::Object(out)
        }
        None => facets.clone(),
    }
}

/// Does a facet object carry at least one clustering key? An empty object ⇒ skip `set_facet` (nothing
/// to cluster), preserving idempotency.
fn facet_is_nonempty(facets: &serde_json::Value) -> bool {
    facets.as_object().is_some_and(|o| !o.is_empty())
}

/// Map an inbound [`Surface`] to the synthesized per-surface emitter marker (`pete@<marker>`, §1b).
/// The HTTP/API surface maps to the web emitter (temperkb.io's surface).
fn surface_marker(s: Surface) -> &'static str {
    match s {
        Surface::CliCloud => "cli",
        Surface::Mcp => "mcp",
        Surface::ApiHttp => "web",
    }
}

/// Split a command's `managed_meta` + `open_meta` into the `(key, value)` property pairs the live write
/// path asserts: Property-fated managed keys (§7) + every open key, dropping nulls. `doc_type` rides the
/// `ResourceCreate` separately, and Die/Edge/ReconcileToDocType managed keys are excluded by fate.
fn properties_from_meta(
    managed_meta: &serde_json::Value,
    open_meta: Option<&serde_json::Value>,
) -> Vec<(String, serde_json::Value)> {
    let mut out: Vec<(String, serde_json::Value)> = Vec::new();
    if let Some(obj) = managed_meta.as_object() {
        for (k, v) in obj {
            if !v.is_null() && key_fate(k) == KeyFate::Property {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    if let Some(obj) = open_meta.and_then(|o| o.as_object()) {
        for (k, v) in obj {
            if !v.is_null() {
                out.push((k.clone(), v.clone()));
            }
        }
    }
    out
}

/// Maps the substrate readback (`readback::resource_row`) to the native `ResourceRow` — real
/// timestamps (event-sourced from `kb_events.occurred_at`), name-only doc type, no fabrication.
/// Shared by `show_resource` and the read selector arms (`list_select`, `show_select`,
/// `search_select`). The §7-dissolved fields (`kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`)
/// are absent; `doc_type_name` is authoritative.
pub(crate) async fn native_resource_row(
    pool: &PgPool,
    principal: uuid::Uuid,
    new_id: uuid::Uuid,
) -> Result<ResourceRow, TemperError> {
    let p = readback::resource_row(pool, principal, new_id)
        .await
        .map_err(map_readback_err)?;
    Ok(ResourceRow {
        id: ResourceId::from(p.re_minted_id),
        kb_context_id: ContextId::from(p.re_minted_context_id),
        origin_uri: p.origin_uri,
        title: p.title,
        originator_profile_id: ProfileId::from(p.originator_profile_id),
        owner_profile_id: ProfileId::from(p.owner_profile_id),
        is_active: p.is_active,
        created: p.created,
        updated: p.updated,
        context_name: p.context_name,
        doc_type_name: p.doc_type_name,
        owner_handle: p.owner_handle,
        stage: p.stage,
        seq: p.seq,
        mode: p.mode,
        effort: p.effort,
        body_hash: p.body_hash,
    })
}

/// The Postgres-backed backend. Holds a pool + the caller profile. The caller's profile id is the
/// substrate principal directly (synthesis preserves profile ids verbatim, WS2); reads/writes are
/// visibility-scoped through `resources_visible_to` / `can_modify_resource`.
pub struct DbBackend {
    pool: PgPool,
    /// The caller profile — the substrate principal directly (a preserved profile id). Reads scope
    /// through `resources_visible_to`; writes gate through `can_modify_resource` (WS2).
    profile_id: ProfileId,
}

/// The invariant attribution carried through every reconcile phase: which cognitive map, on whose
/// behalf (`owner`), and under which event `emitter`. Bundled so each phase helper takes one context
/// argument instead of threading three identical ids — and to stay under the params-struct threshold.
#[derive(Clone, Copy, Debug)]
struct ReconcileCtx {
    cogmap: temper_substrate::ids::CogmapId,
    owner: temper_substrate::ids::ProfileId,
    emitter: temper_substrate::ids::EntityId,
}

impl DbBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId) -> Self {
        Self { pool, profile_id }
    }

    /// Auth-before-writes gate (WS2): the caller (`self.profile_id`, the substrate principal directly)
    /// must be able to modify the target resource. Returns `Forbidden` otherwise. CONFORMs to
    /// production's `check_can_modify`. `can_modify_resource` and its nested `profile_effective_teams`/
    /// `team_ancestors` resolve their unqualified references against the connection search_path (the one
    /// schema post-collapse), so no per-txn `SET LOCAL`.
    async fn check_can_modify_next(&self, new_id: uuid::Uuid) -> Result<(), TemperError> {
        let can: Option<bool> = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
            .bind(*self.profile_id)
            .bind(new_id)
            .fetch_one(&self.pool)
            .await
            .map_err(api_err)?;
        if can.unwrap_or(false) {
            Ok(())
        } else {
            Err(TemperError::Forbidden)
        }
    }

    /// The source resource an edge mutation is authorized against. Production gates edge
    /// retype/reweight/fold on "can modify the SOURCE resource" (`handlers::edges` → 403 "Cannot modify
    /// source resource"); the parity-era write path only ever asserts resource→resource edges, so the
    /// source is always a `kb_resources` endpoint.
    async fn edge_source_resource(&self, edge_id: uuid::Uuid) -> Result<uuid::Uuid, TemperError> {
        sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT source_id FROM kb_edges \
             WHERE id = $1 AND source_table = 'kb_resources'",
        )
        .bind(edge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        .ok_or_else(|| TemperError::NotFound(format!("edge {edge_id} not found")))
    }

    /// The diff+apply core of `reconcile_cognitive_map`, run INSIDE the `admin_reconcile` envelope on a
    /// caller-supplied serializable transaction connection (`conn`). Returns the [`ReconcileOutcome`];
    /// any `Err` is propagated and the caller drops the transaction → Postgres rolls back EVERYTHING
    /// (every mutation AND the envelope open), so a partial reconcile is structurally impossible.
    ///
    /// The diff keys on the STABLE landmark `id` (the entry's pre-generated uuidv7), NEVER on
    /// `origin_uri` (which stays as loose, non-unique attribution). Idempotency (the headline invariant)
    /// holds because: an entry whose body merkle equals the live `body_hash` does NOTHING (zero events);
    /// an edge is asserted only when ABSENT (checked via a polarity-aware `find_edge`); the clustering
    /// facet + `provenance` stamp are written only on CREATE. Re-running the same request therefore fires
    /// zero new mutation events. (Facet-delta + edge-reweight on EXISTING entries are DEFERRED for v1 —
    /// kernel landmarks are born with their facets/edges and rarely change them; re-asserting either
    /// appends an event unconditionally, which would break idempotency.)
    ///
    /// Edge endpoints are stable ids needing no resolution (`entry.id` source, `edge.to` target);
    /// `reconcile_cognitive_map` pre-flight-validates every `edge.to` against the ids of
    /// `request.entries` ∪ the live slice BEFORE opening the transaction, so an unresolved target is a
    /// hard `BadRequest` with no writes — not a silent skip.
    async fn reconcile_apply(
        &self,
        conn: &mut sqlx::PgConnection,
        cogmap: temper_substrate::ids::CogmapId,
        request: &ReconcileCogmapRequest,
        owner: temper_substrate::ids::ProfileId,
        emitter: temper_substrate::ids::EntityId,
    ) -> Result<ReconcileOutcome, TemperError> {
        let ctx = ReconcileCtx {
            cogmap,
            owner,
            emitter,
        };

        // The diff source: the live `provenance: kernel` slice, indexed by the STABLE resource id
        // (the diff key — `origin_uri` is loose, non-unique attribution, NEVER a key). Read on the
        // SAME transaction so the diff sees a consistent snapshot under SERIALIZABLE.
        let live_by_id = Self::read_kernel_index(&mut *conn, cogmap.uuid()).await?;

        let mut outcome = ReconcileOutcome::default();

        // Phase order is load-bearing: resources first (an edge's target must already exist), then
        // edges, then explicit tombstones. Every phase runs on the one transaction — any `Err`
        // propagates and the caller drops the tx, so Postgres rolls back the WHOLE reconcile (every
        // mutation AND the envelope open). A partial reconcile is structurally impossible.
        Self::apply_resource_phase(&mut *conn, request, &live_by_id, ctx, &mut outcome).await?;
        Self::apply_edge_phase(&mut *conn, request, ctx).await?;
        Self::apply_tombstone_phase(&mut *conn, request, &live_by_id, ctx, &mut outcome).await?;

        Ok(outcome)
    }

    /// Read the live `provenance: kernel` slice and index it by the STABLE resource id — the diff
    /// key. (`origin_uri` is loose, non-unique attribution and is NEVER a key.) Owns the rows so the
    /// index outlives the read without borrowing the source vec.
    async fn read_kernel_index(
        conn: &mut sqlx::PgConnection,
        cogmap_uuid: uuid::Uuid,
    ) -> Result<std::collections::HashMap<uuid::Uuid, readback::KernelSliceRow>, TemperError> {
        let live = readback::kernel_slice(&mut *conn, cogmap_uuid)
            .await
            .map_err(api_err)?;
        Ok(live.into_iter().map(|r| (r.resource_id, r)).collect())
    }

    /// PHASE 1 — resources (create / update / no-op). NO edges yet (targets may not exist). Keyed on
    /// the stable id via `live_by_id`; the merkle compare drives create/update/unchanged.
    async fn apply_resource_phase(
        conn: &mut sqlx::PgConnection,
        request: &ReconcileCogmapRequest,
        live_by_id: &std::collections::HashMap<uuid::Uuid, readback::KernelSliceRow>,
        ctx: ReconcileCtx,
        outcome: &mut ReconcileOutcome,
    ) -> Result<(), TemperError> {
        for entry in &request.entries {
            // Unpack the supplied (client-embedded) chunks once. The body merkle the substrate WILL
            // store for them is computed the SAME way the create-dedup path does
            // (`body_hash_from_chunk_hashes`), so it byte-matches the stored `body_hash`. The
            // diff keys on THIS merkle — never the wire `content_hash`, which the CLI derives
            // differently (whole-body `sha256:`-prefixed hash, not the chunk-merkle) and which the
            // server therefore does not trust. Trusting it would make every re-run re-block every entry.
            let incoming_chunks = unpack_incoming_chunks(&entry.chunks_packed)?;
            let chunk_hashes: Vec<String> = incoming_chunks
                .iter()
                .map(|c| c.content_hash.clone())
                .collect();
            let incoming_body_hash =
                temper_substrate::content::body_hash_from_chunk_hashes(&chunk_hashes);

            match live_by_id.get(&entry.id) {
                None => {
                    // CREATE — the resource itself (minted under the STABLE landmark id), then STAMP
                    // provenance, then the clustering facets. `body` is empty: the reconcile wire carries
                    // no raw prose — `chunks` is always `Some` here, so the wrapper builds the block from
                    // the chunks and ignores `body` (the `body` param is only the no-chunks server-embed
                    // fallback, never taken here). `origin_uri` is still set on the resource as
                    // attribution.
                    let chunks = Some(incoming_chunks);
                    let rid = writes::create_kernel_resource_in_tx(
                        &mut *conn,
                        writes::KernelCreateParams {
                            cogmap: ctx.cogmap,
                            resource_id: entry.id,
                            title: &entry.title,
                            origin_uri: &entry.origin_uri,
                            doc_type: &entry.doc_type,
                            body: "",
                            chunks,
                            owner: ctx.owner,
                            emitter: ctx.emitter,
                        },
                    )
                    .await
                    .map_err(api_err)?;

                    // STAMP `provenance: kernel` — the per-key property `kernel_slice` filters on
                    // (Decision #6); every reconcile-created resource is kernel by definition.
                    writes::set_property_in_tx(
                        &mut *conn,
                        rid,
                        "provenance",
                        &serde_json::json!("kernel"),
                        ctx.emitter,
                    )
                    .await
                    .map_err(api_err)?;

                    // Clustering facets (e.g. `layer`) — strip any stray `provenance` (stamped above,
                    // never clustered). Skip the write entirely when there's nothing to cluster.
                    let facets = strip_provenance_facet(&entry.facets);
                    if facet_is_nonempty(&facets) {
                        writes::set_facet_in_tx(&mut *conn, rid, &facets, 1.0, ctx.emitter)
                            .await
                            .map_err(api_err)?;
                    }

                    // `rid` equals `entry.id` (the create minted under it); the diff already keys edges
                    // on `entry.id`, so there is no id-by-uri map to maintain.
                    debug_assert_eq!(rid.uuid(), entry.id);
                    outcome.created += 1;
                }
                Some(row) if row.body_hash.as_deref() != Some(incoming_body_hash.as_str()) => {
                    // UPDATE — body changed (the stored merkle differs from the incoming chunks'
                    // merkle). Re-block from the supplied chunks. (Facet/edge deltas on an existing
                    // entry are DEFERRED v1 — see the method doc.)
                    writes::update_resource_in_tx(
                        &mut *conn,
                        writes::UpdateParams {
                            resource: temper_substrate::ids::ResourceId::from(row.resource_id),
                            // `Some("")` requests a body re-block (the re-block fires iff body is
                            // `Some`); the content comes from `chunks` (always `Some` here), so the
                            // empty string is never embedded — see the CREATE arm's note.
                            body: Some(""),
                            title: None,
                            origin_uri: None,
                            properties: &[],
                            chunks: Some(incoming_chunks),
                            rehome_to: None,
                            emitter: ctx.emitter,
                        },
                    )
                    .await
                    .map_err(api_err)?;
                    outcome.updated += 1;
                }
                Some(_) => {
                    // Hashes equal → DO NOTHING (zero events). The idempotency invariant.
                    outcome.unchanged += 1;
                }
            }
        }
        Ok(())
    }

    /// PHASE 2 — edges (idempotent: assert only those not already present). Both endpoints are stable
    /// landmark ids (`entry.id` source, `edge.to` target) — NO lookup; pre-flight already proved each
    /// `edge.to` resolves to a kernel resource. Edges never touch `outcome` (no edge counter by design).
    async fn apply_edge_phase(
        conn: &mut sqlx::PgConnection,
        request: &ReconcileCogmapRequest,
        ctx: ReconcileCtx,
    ) -> Result<(), TemperError> {
        for entry in &request.entries {
            let src = entry.id;
            for e in &entry.edges {
                let tgt = e.to;
                let kind =
                    temper_substrate::affinity::EdgeKind::from_sql(&e.kind).ok_or_else(|| {
                        TemperError::BadRequest(format!("unknown edge kind: {}", e.kind))
                    })?;
                let polarity = temper_substrate::payloads::EdgePolarity::from_sql(&e.polarity)
                    .ok_or_else(|| {
                        TemperError::BadRequest(format!("unknown edge polarity: {}", e.polarity))
                    })?;
                // Per-edge existence check, polarity-aware (a forward and an inverse edge of the same
                // kind to the same target are distinct, both deliverable). Re-asserting a present edge
                // would append an event (`relationship_assert` fires unconditionally) and break
                // idempotency, so skip when already present. The kernel has ~15 edges; per-edge
                // `find_edge` is fine.
                let present = readback::find_edge(&mut *conn, src, tgt, &kind, Some(&e.polarity))
                    .await
                    .map_err(api_err)?
                    .is_some();
                if present {
                    continue; // already present — re-assert would break idempotency
                }
                writes::assert_kernel_edge_in_tx(
                    &mut *conn,
                    writes::KernelEdgeParams {
                        cogmap: ctx.cogmap,
                        src: temper_substrate::ids::ResourceId::from(src),
                        tgt: temper_substrate::ids::ResourceId::from(tgt),
                        kind,
                        polarity,
                        label: e.label.as_deref(),
                        weight: e.weight,
                        emitter: ctx.emitter,
                    },
                )
                .await
                .map_err(api_err)?;
            }
        }
        Ok(())
    }

    /// PHASE 3 — explicit tombstones only (O3: absence alone NEVER folds). Keyed on the stable id.
    async fn apply_tombstone_phase(
        conn: &mut sqlx::PgConnection,
        request: &ReconcileCogmapRequest,
        live_by_id: &std::collections::HashMap<uuid::Uuid, readback::KernelSliceRow>,
        ctx: ReconcileCtx,
        outcome: &mut ReconcileOutcome,
    ) -> Result<(), TemperError> {
        for t in &request.fold_resources {
            if let Some(row) = live_by_id.get(&t.id) {
                writes::delete_resource_in_tx(
                    &mut *conn,
                    temper_substrate::ids::ResourceId::from(row.resource_id),
                    ctx.emitter,
                )
                .await
                .map_err(api_err)?;
                outcome.folded += 1;
            }
        }
        for t in &request.fold_edges {
            let kind = temper_substrate::affinity::EdgeKind::from_sql(&t.kind)
                .ok_or_else(|| TemperError::BadRequest(format!("unknown edge kind: {}", t.kind)))?;
            // Resolve the live edge by (from, to, kind) over `kb_edges` (any polarity → `None`) —
            // substrate SQL lives in the substrate (`readback::find_edge`), run on this transaction's
            // connection.
            let edge_id = readback::find_edge(&mut *conn, t.from, t.to, &kind, None)
                .await
                .map_err(api_err)?;
            if let Some(edge_id) = edge_id {
                writes::fold_relationship_in_tx(
                    &mut *conn,
                    temper_substrate::ids::EdgeId::from(edge_id),
                    Some("reconcile fold"),
                    ctx.emitter,
                )
                .await
                .map_err(api_err)?;
                outcome.folded += 1;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Backend for DbBackend {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        // Resolve the caller's synthesized identity (natural-key) and the home context.
        let prod_profile: uuid::Uuid = *self.profile_id;
        let owner = writes::resolve_profile(&self.pool, prod_profile)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let home = writes::resolve_context(&self.pool, owner, &cmd.context)
            .await
            .map_err(api_err)?;

        let body = cmd
            .body
            .as_ref()
            .map(|b| b.content.clone())
            .unwrap_or_default();
        let origin_uri = cmd.origin_uri.clone().unwrap_or_default();

        // Honor caller-supplied precomputed chunks (the client did extract→chunk→embed). When present
        // the server carries the vectors verbatim — no server-side ONNX — and keys dedup on the merkle
        // of the SUPPLIED chunk hashes; when absent the server chunks + embeds `body` itself (the
        // fallback). Reverses PR#71's "server is the single source of truth" discard contract. A
        // malformed blob is a caller fault → BadRequest (propagated, never swallowed).
        let incoming_chunks: Option<Vec<temper_substrate::content::IncomingChunk>> =
            match &cmd.chunks_packed {
                Some(packed) => Some(unpack_incoming_chunks(packed)?),
                None => None,
            };

        // Create-time guards (WS6 collapse Task F). The substrate create path applies the same
        // strip → defaults → identity-keys → validate → body-hash-dedup pipeline the legacy
        // `ingest_service::ingest` ran (`:433-502`), calling the surviving pure helpers WHERE THEY
        // LIVE (no helper move — the flip owns relocation). Purely additive over the prior no-guard
        // create.
        let mut managed =
            serde_json::to_value(&cmd.managed_meta).map_err(|e| TemperError::Api(e.to_string()))?;
        // 1. Strip identity / tier-1 system keys a caller may have echoed back from a prior read.
        managed = temper_workflow::operations::strip_system_managed_fields(managed);
        // 2. Apply doc-type managed-tier defaults (e.g. task → `temper-stage: backlog`).
        temper_workflow::operations::apply_defaults_value(&cmd.doctype, &mut managed);
        // 3. Inject the canonical identity keys (`temper-title`/`temper-slug`) before validation, the
        //    same send/receive-symmetric discipline ingest uses. An empty slug removes `temper-slug`
        //    (mirrors ingest's `injected_slug` at `:444-453`).
        let injected_slug = (!cmd.slug.is_empty()).then_some(cmd.slug.as_str());
        temper_workflow::operations::ensure_managed_identity_keys(
            &mut managed,
            &cmd.title,
            injected_slug,
        );
        // 4. Validate the assembled managed_meta against the doc-type schema; PROPAGATE the typed
        //    validation error (never swallow it). A fresh canonical id + `now()` seed the validation
        //    document exactly as ingest does (`:457-467`); that id is not persisted from here — the
        //    substrate mints the resource id in `writes::create_resource`.
        let validate_params = temper_workflow::operations::ValidateManagedMetaParams {
            id: uuid::Uuid::now_v7(),
            created: Utc::now(),
            doc_type: &cmd.doctype,
            managed_meta: Some(&managed),
            slug: &cmd.slug,
            title: &cmd.title,
            context_name: &cmd.context,
        };
        // `validate_managed_meta` returns a typed `TemperError::BadRequest` on a caller-input fault;
        // propagate it directly (PROPAGATE, never swallow).
        temper_workflow::operations::validate_managed_meta(&validate_params)?;

        // 5. Body-hash dedup (non-empty body only, matching legacy `:497-502`): if a visible active
        //    resource already carries the same substrate body_hash merkle, return IT instead of
        //    creating a twin — reconstructing the same `CommandOutput<ResourceRow>` the create path
        //    returns for a fresh row (`:254-255`).
        if !body.is_empty() {
            // Key dedup on the SUPPLIED chunk hashes when the caller pre-chunked (so it equals the
            // body_hash the substrate projector will store from those same hashes); otherwise on the
            // chunk-the-prose merkle (the fallback).
            let body_hash = match &incoming_chunks {
                Some(chunks) => {
                    let hashes: Vec<String> =
                        chunks.iter().map(|c| c.content_hash.clone()).collect();
                    temper_substrate::content::body_hash_from_chunk_hashes(&hashes)
                }
                None => temper_substrate::content::body_hash_for_body(&body),
            };
            if let Some(existing) =
                readback::find_by_body_hash(&self.pool, *self.profile_id, &body_hash)
                    .await
                    .map_err(api_err)?
            {
                let row = native_resource_row(&self.pool, *self.profile_id, existing).await?;
                return Ok(CommandOutput::new(row));
            }
        }

        let properties = properties_from_meta(&managed, cmd.open_meta.as_ref());

        let new_id = writes::create_resource(
            &self.pool,
            writes::CreateParams {
                title: &cmd.title,
                origin_uri: &origin_uri,
                body: &body,
                doc_type: &cmd.doctype,
                home,
                owner,
                // A fresh create's originator is its owner (the caller); a distinct originator only
                // arises via synthesis carrying a production row's history.
                originator: owner,
                emitter,
                properties: &properties,
                chunks: incoming_chunks,
            },
        )
        .await
        .map_err(api_err)?;

        let row = native_resource_row(&self.pool, *self.profile_id, new_id.uuid()).await?;
        Ok(CommandOutput::new(row))
    }

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        // The inbound id IS the substrate resource id — synthesis preserves resource ids verbatim, so
        // there is no origin_uri remap (the prior bimap collapsed empty-origin_uri resources onto one id).
        let new_id = uuid::Uuid::from(cmd.resource);
        // `native_resource_row` gates visibility (WS2) and maps the typed `ReadbackError` via
        // `map_readback_err`: not-visible → NotFound (404, the leak-safe deny — never 403, no
        // existence-leak oracle), a genuine fault → Api (500). The earlier blanket `|_| NotFound`
        // collapse masked real faults as 404.
        let row = native_resource_row(&self.pool, *self.profile_id, new_id).await?;
        Ok(CommandOutput::new(row))
    }

    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        // The inbound id IS the substrate resource id (native-id addressing — synthesis carries
        // resource ids verbatim, so no origin_uri remap).
        let new_id = uuid::Uuid::from(cmd.resource);
        // Auth before any write (WS2): the caller must be able to modify this resource.
        self.check_can_modify_next(new_id).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;

        let body = cmd.body.as_ref().map(|b| b.content.clone());
        // Honor caller-supplied precomputed chunks on the revise too (client did extract→chunk→embed):
        // carry the vectors verbatim instead of re-embedding server-side. Absent ⇒ server chunks +
        // embeds `body` (fallback). Reverses PR#71's discard contract.
        let incoming_chunks: Option<Vec<temper_substrate::content::IncomingChunk>> =
            match cmd.body.as_ref().and_then(|b| b.chunks_packed.as_deref()) {
                Some(packed) => Some(unpack_incoming_chunks(packed)?),
                None => None,
            };
        // temper-title (a §7-Die managed key) maps to the kb_resources.title column, not a property.
        let mut title: Option<String> = None;
        let mut properties: Vec<(String, serde_json::Value)> = Vec::new();
        if let Some(mm) = &cmd.managed_meta {
            let incoming = serde_json::to_value(mm).map_err(|e| TemperError::Api(e.to_string()))?;

            // Update-time validation (mirror create's strip→defaults→identity→validate
            // pipeline; restores the legacy `resource_service::update` guard the collapse
            // dropped). The wrinkle vs create: update carries no doc_type/context/slug, so
            // take the EFFECTIVE values from the current row (legacy did the same — load
            // `current`, `effective_doc_type = incoming.unwrap_or(&current.doc_type_name)`).
            // The current row is reconstructed for these values (already visibility-gated
            // via `check_can_modify_next`).
            let current = native_resource_row(&self.pool, *self.profile_id, new_id).await?;
            // A type change arrives as `temper-type` in managed_meta (the PUT /meta path) or
            // `move_to.type_to` (the file-move path); else the doc type is unchanged.
            let effective_doc_type = incoming
                .get("temper-type")
                .and_then(|v| v.as_str())
                .or_else(|| cmd.move_to.as_ref().and_then(|m| m.type_to.as_deref()))
                .unwrap_or(current.doc_type_name.as_str())
                .to_owned();
            let effective_context = cmd
                .move_to
                .as_ref()
                .and_then(|m| m.context_to.as_deref())
                .unwrap_or(current.context_name.as_str())
                .to_owned();
            // temper-title updates the kb_resources.title column when supplied; otherwise the
            // current title carries (and seeds validation). temper-slug is §7-Die (not stored,
            // so `current.slug` is None) — derive the canonical slug from the title so the
            // `temper-slug`-required schemas don't FALSE-reject a valid update.
            let incoming_title = incoming
                .get("temper-title")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let effective_title = incoming_title
                .clone()
                .unwrap_or_else(|| current.title.clone());
            let effective_slug = temper_workflow::operations::sluggify(&effective_title);

            // Build the COMPLETE validation document (strip system keys → doc-type defaults →
            // identity keys) and validate it; PROPAGATE the typed BadRequest (an out-of-enum
            // value or an unknown doc_type → 400, the create contract). Every schema-required
            // field is supplied by identity (temper-slug/title) or a default (task temper-stage /
            // goal temper-status), so a partial update never false-rejects — no merge with the
            // current managed_meta is needed.
            let mut validation =
                temper_workflow::operations::strip_system_managed_fields(incoming.clone());
            temper_workflow::operations::apply_defaults_value(&effective_doc_type, &mut validation);
            temper_workflow::operations::ensure_managed_identity_keys(
                &mut validation,
                &effective_title,
                Some(effective_slug.as_str()),
            );
            let validate_params = temper_workflow::operations::ValidateManagedMetaParams {
                id: new_id,
                created: current.created,
                doc_type: &effective_doc_type,
                managed_meta: Some(&validation),
                slug: &effective_slug,
                title: &effective_title,
                context_name: &effective_context,
            };
            temper_workflow::operations::validate_managed_meta(&validate_params)?;

            // Write only the caller-supplied keys (PATCH is a partial merge; `PropertySet`
            // asserts per key, so unsupplied keys are untouched — DON'T write the defaulted
            // validation set). `properties_from_meta` filters to §7-Property keys, so the
            // §7-Die identity keys + the §7-ReconcileToDocType `temper-type` never become rows.
            title = incoming_title;
            properties = properties_from_meta(&incoming, cmd.open_meta.as_ref());
        } else if cmd.open_meta.is_some() {
            properties = properties_from_meta(&serde_json::Value::Null, cmd.open_meta.as_ref());
        }

        // A type-move sets the authoritative `doc_type` property; a context-move re-homes.
        let mut rehome_to = None;
        if let Some(mv) = &cmd.move_to {
            if let Some(type_to) = &mv.type_to {
                properties.push(("doc_type".to_owned(), serde_json::json!(type_to)));
            }
            if let Some(ctx_to) = &mv.context_to {
                rehome_to = Some(
                    writes::resolve_context(&self.pool, owner, ctx_to)
                        .await
                        .map_err(api_err)?,
                );
            }
        }

        writes::update_resource(
            &self.pool,
            writes::UpdateParams {
                resource: temper_substrate::ids::ResourceId::from(new_id),
                body: body.as_deref(),
                title: title.as_deref(),
                origin_uri: None,
                properties: &properties,
                chunks: incoming_chunks,
                rehome_to,
                emitter,
            },
        )
        .await
        .map_err(api_err)?;

        let row = native_resource_row(&self.pool, *self.profile_id, new_id).await?;
        Ok(CommandOutput::new(row))
    }

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError> {
        // The inbound id IS the substrate resource id (no origin_uri remap).
        let new_id = uuid::Uuid::from(cmd.resource);
        // Auth before any write (WS2).
        self.check_can_modify_next(new_id).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::delete_resource(
            &self.pool,
            temper_substrate::ids::ResourceId::from(new_id),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(()))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let rows = readback::list(&self.pool, *self.profile_id)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        let summaries = rows
            .into_iter()
            .map(|r| ResourceSummary {
                // slug is §7-dissolved; the list summary uses origin_uri as the stable handle.
                slug: r.origin_uri,
                doctype: r.doc_type,
                // Context scoping for the list summary (WS2); unscoped at the §9 floor.
                context: String::new(),
                title: r.title,
            })
            .collect();
        Ok(CommandOutput::new(summaries))
    }

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        // 4b: FTS only (the text query). Vector search needs a query embedding this layer does not
        // carry; the HTTP search selector handles vector mode directly (read selector, Task 6/8).
        // `fts_search` returns the preserved resource ids (origin_uri is non-unique — empty for
        // CLI/agent-created resources, so it cannot identify a match). Each id reconstructs to its
        // summary (origin_uri verbatim as the stable handle, like `list_resources`).
        let ids = readback::fts_search(&self.pool, *self.profile_id, &cmd.query.query)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        let mut hits = Vec::with_capacity(ids.len());
        for new_id in ids {
            let row = native_resource_row(&self.pool, *self.profile_id, new_id).await?;
            hits.push(SearchHit {
                summary: ResourceSummary {
                    // slug is §7-dissolved; the summary uses origin_uri as the stable handle.
                    slug: row.origin_uri,
                    doctype: row.doc_type_name,
                    context: row.context_name,
                    title: row.title,
                },
                // §9 floor asserts the matching SET, not the score.
                score: 0.0,
            });
        }
        Ok(CommandOutput::new(hits))
    }

    async fn assert_relationship(
        &self,
        cmd: AssertRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Source and target ids ARE the substrate resource ids (synthesis carries resource ids verbatim)
        // — used directly, no origin_uri remap (the prior bimap collapsed empty-origin_uri resources
        // onto one arbitrary id).
        let src_next = uuid::Uuid::from(cmd.source);
        // Auth before any write (WS2): edge mutations gate on the SOURCE resource (production's
        // "Cannot modify source resource"). Gate before resolving the target / writing the edge.
        self.check_can_modify_next(src_next).await?;

        let tgt_next = uuid::Uuid::from(cmd.target);

        // The edge homes in the source's substrate context (its home anchor).
        let home_next: uuid::Uuid = sqlx::query_scalar(
            "SELECT anchor_id FROM kb_resource_homes \
             WHERE resource_id=$1 AND anchor_table='kb_contexts'",
        )
        .bind(src_next)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;

        let label = (!cmd.label.is_empty()).then_some(cmd.label.as_str());
        let edge = writes::assert_relationship(
            &self.pool,
            writes::AssertParams {
                src: temper_substrate::ids::ResourceId::from(src_next),
                tgt: temper_substrate::ids::ResourceId::from(tgt_next),
                kind: map_edge_kind(cmd.edge_kind),
                polarity: map_polarity(cmd.polarity),
                label,
                weight: cmd.weight,
                home: temper_substrate::ids::ContextId::from(home_next),
                emitter,
            },
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(edge.uuid()))
    }

    async fn retype_relationship(
        &self,
        cmd: RetypeRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // The edge handle on the substrate backend IS the substrate edge id (returned by assert).
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.edge_handle).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::retype_relationship(
            &self.pool,
            temper_substrate::ids::EdgeId::from(cmd.edge_handle),
            map_edge_kind(cmd.edge_kind),
            map_polarity(cmd.polarity),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    async fn reweight_relationship(
        &self,
        cmd: ReweightRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.edge_handle).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::reweight_relationship(
            &self.pool,
            temper_substrate::ids::EdgeId::from(cmd.edge_handle),
            cmd.weight,
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    async fn fold_relationship(
        &self,
        cmd: FoldRelationship,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(cmd.edge_handle).await?;
        self.check_can_modify_next(src).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        writes::fold_relationship(
            &self.pool,
            temper_substrate::ids::EdgeId::from(cmd.edge_handle),
            cmd.reason.as_deref(),
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    /// One idempotent desired-state reconcile run as a SINGLE `SERIALIZABLE` transaction: the
    /// `admin_reconcile` envelope open, every kernel mutation, and the envelope close all commit
    /// atomically (the system actor fires every mutation). Atomicity makes a half-open envelope
    /// structurally impossible — any error before commit drops the transaction → Postgres rolls back
    /// EVERYTHING (mutations + the open), so there is no Failed-close path and no stale-open lock.
    /// SERIALIZABLE makes concurrent reconciles abort-and-retry (SQLSTATE 40001 → `Conflict`) instead of
    /// corrupting state — the old app-level open-invocation "mutex" is gone. No HTTP/authz here (the
    /// handler gates first); this is the backend command.
    async fn reconcile_cognitive_map(
        &self,
        cmd: ReconcileCognitiveMap,
    ) -> Result<CommandOutput<ReconcileOutcome>, TemperError> {
        let cogmap = temper_substrate::ids::CogmapId::from(cmd.cogmap_id);

        // The system actor: every kernel mutation fires under (owner = system profile, emitter = system
        // entity) — the L0 birth migration's actor.
        let (owner, emitter) = readback::system_actor(&self.pool).await.map_err(api_err)?;

        // PRE-FLIGHT (FIX #3) — fail fast + loud on an unresolved edge target, BEFORE opening the
        // transaction, so a bad manifest writes NOTHING. A quick read on the pool; the authoritative
        // in-tx read still happens inside `reconcile_apply`.
        self.preflight_validate_edge_targets(cmd.cogmap_id, &cmd.request)
            .await?;

        // ONE SERIALIZABLE transaction for the whole run. `SET TRANSACTION ISOLATION LEVEL SERIALIZABLE`
        // must precede any query in the transaction — it is the first statement after BEGIN.
        let mut tx = self.pool.begin().await.map_err(api_err)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(api_err)?;

        // OPEN the envelope (top-level: `parent: None`, so the delegation gate is not exercised).
        let inv = writes::open_invocation_in_tx(
            &mut tx,
            writes::OpenParams {
                trigger_kind: "admin_reconcile".to_string(),
                originating: cogmap,
                parent: None,
                scoped_entity: emitter,
                emitter,
            },
        )
        .await
        .map_err(api_err)?;

        // APPLY on the same connection. On ANY error the `?` returns here with `tx` dropped → full
        // rollback (mutations + the envelope open). No Failed-close needed.
        let outcome = self
            .reconcile_apply(&mut tx, cogmap, &cmd.request, owner, emitter)
            .await?;

        // CLOSE the envelope `Completed` in the same transaction.
        let outcome_json =
            serde_json::to_value(&outcome).map_err(|e| TemperError::Api(e.to_string()))?;
        writes::close_invocation_in_tx(
            &mut tx,
            inv,
            cogmap,
            temper_substrate::payloads::Disposition::Completed,
            outcome_json,
            emitter,
        )
        .await
        .map_err(api_err)?;

        // COMMIT — a serialization failure (40001) maps to `Conflict` (retryable), any other DB error to
        // a 500. Success returns the outcome.
        match tx.commit().await {
            Ok(()) => Ok(CommandOutput::new(outcome)),
            Err(e) => Err(map_commit_err(e)),
        }
    }
}

/// Pre-flight validation (FIX #3): every reconcile edge target must resolve to a kernel resource that
/// either already exists (the live slice) or is being created/kept this run (`request.entries`).
impl DbBackend {
    async fn preflight_validate_edge_targets(
        &self,
        cogmap_uuid: uuid::Uuid,
        request: &ReconcileCogmapRequest,
    ) -> Result<(), TemperError> {
        use std::collections::HashSet;

        let live = readback::kernel_slice(&self.pool, cogmap_uuid)
            .await
            .map_err(api_err)?;

        // The resolvable set: stable ids of live resources ∪ this request's entry ids.
        let mut known: HashSet<uuid::Uuid> = HashSet::new();
        for r in &live {
            known.insert(r.resource_id);
        }
        for e in &request.entries {
            known.insert(e.id);
        }

        let unresolved = |id: uuid::Uuid| {
            TemperError::BadRequest(format!(
                "reconcile: edge target {id} resolves to no kernel resource"
            ))
        };

        // Every outgoing edge's target must resolve.
        for entry in &request.entries {
            for edge in &entry.edges {
                if !known.contains(&edge.to) {
                    return Err(unresolved(edge.to));
                }
            }
        }
        // Every fold_edges endpoint (both ends) must resolve.
        for t in &request.fold_edges {
            if !known.contains(&t.from) {
                return Err(unresolved(t.from));
            }
            if !known.contains(&t.to) {
                return Err(unresolved(t.to));
            }
        }
        Ok(())
    }
}

/// Map a `tx.commit()` error: a SERIALIZABLE serialization failure (SQLSTATE `40001`) is a concurrent-
/// reconcile conflict → retryable [`TemperError::Conflict`]; any other DB error is a 500 ([`api_err`]).
fn map_commit_err(e: sqlx::Error) -> TemperError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("40001") {
            return TemperError::Conflict(
                "reconcile conflicted with a concurrent run; retry".to_string(),
            );
        }
    }
    api_err(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_object_safe() {
        fn assert_obj(_: &dyn Backend) {}
        let _ = assert_obj;
        // If DbBackend were not object-safe, the boxed `dyn Backend` dispatch would not compile.
    }
}
