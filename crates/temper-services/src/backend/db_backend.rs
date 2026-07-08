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
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{
    CogmapId, ContextId, EdgeId, EntityId, InvocationId, ProfileId, PropertyId, ResourceId,
};
use temper_core::types::materialize::{
    MaterializeAck, DEFAULT_MATERIALIZE_LENS, DEFAULT_MATERIALIZE_THRESHOLD,
};
use temper_core::types::reconcile::{
    CharterDisposition, CreateCogmapOutcome, ReconcileCogmapRequest, ReconcileOutcome,
    ReconcileTelos,
};
use temper_core::types::workflow_job::{DispatchType, Persona};
use temper_substrate::payloads::AnchorRef;
use temper_workflow::operations::{
    AdvanceStewardWatermark, AssertRelationship, Backend, BodyUpdate, CloseInvocation,
    CommandOutput, CreateCognitiveMap, CreateResource, DeleteResource, FoldRelationship, GoalPatch,
    ListResources, MaterializeOnThreshold, OpenInvocation, ReconcileCognitiveMap, ResourceSummary,
    RetypeRelationship, ReweightRelationship, SearchHit, SearchResources, SetFacet, ShowResource,
    StewardDispatchTick, Surface, UpdateResource,
};
use temper_workflow::types::resource::ResourceRow;

use temper_substrate::content::PreparedBlock;
use temper_substrate::events::{fire_with, EventContext, SeedAction};
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

/// The relationship label for a resource→goal edge — "this resource *advances* that goal",
/// mirroring the session→task `advances` link (see `link_session_to_task`) one level up.
/// Load-bearing: it is part of the edge idempotency key AND the `list --goal` filter
/// predicate, so the create/update projection, the fold-lookup, and the list SQL must all
/// agree on this exact string. `pub(crate)` so the `list --goal` filter
/// (`substrate_read::filtered_visible_page`) binds the same label.
pub(crate) const GOAL_EDGE_LABEL: &str = "advances";

/// The endpoints + shape of an edge to assert from a source resource's home anchor
/// (see `DbBackend::assert_edge_from_source_home`). A params struct so the helper stays
/// within the argument-count budget (preferred over `#[expect(too_many_arguments)]`).
struct SourceHomedEdge<'a> {
    src: uuid::Uuid,
    tgt: uuid::Uuid,
    edge_kind: graph::EdgeKind,
    polarity: graph::Polarity,
    label: &'a str,
    weight: f64,
    emitter: EntityId,
}

/// graph::Polarity → temper-substrate's payloads::EdgePolarity.
fn map_polarity(p: graph::Polarity) -> temper_substrate::payloads::EdgePolarity {
    use temper_substrate::payloads::EdgePolarity as N;
    match p {
        graph::Polarity::Forward => N::Forward,
        graph::Polarity::Inverse => N::Inverse,
    }
}

/// temper-core's wire `Disposition` → temper-substrate's `payloads::Disposition` (identical 3-variant
/// terminal taxonomy). Exhaustive match (the `map_edge_kind` pattern), NOT a stringly conversion:
/// temper-core does not depend on temper-substrate, so the two mirror enums are bridged here.
fn map_disposition(
    d: temper_core::types::invocation::Disposition,
) -> temper_substrate::payloads::Disposition {
    use temper_core::types::invocation::Disposition as Core;
    use temper_substrate::payloads::Disposition as Sub;
    match d {
        Core::Completed => Sub::Completed,
        Core::Failed => Sub::Failed,
        Core::Abandoned => Sub::Abandoned,
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

/// A body update's declared provenance sources → substrate `Incorporation`s, with each source's
/// position in the list becoming its accretion `seq`. `None` (metadata-only update, or no body) and
/// an empty `sources` both yield no incorporations — the projector then writes no `kb_block_provenance`
/// rows. Shared by the create and update paths so both derive `seq` identically.
fn body_sources(body: Option<&BodyUpdate>) -> Vec<temper_substrate::payloads::Incorporation> {
    body.map(|b| {
        b.sources
            .iter()
            .enumerate()
            .map(|(i, source)| temper_substrate::payloads::Incorporation {
                source: source.clone(),
                seq: i as i32,
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Build the substrate-native charter [`PreparedBlock`]s from a wire [`ReconcileTelos`] (client-embedded
/// chunks, carried verbatim — NO server-side ONNX). One block per role-tagged entry, seq-ordered by
/// position. Shared by `apply_telos_phase` (reconcile) and `create_cognitive_map` (genesis) so the two
/// build charter blocks identically.
fn prepare_telos_blocks(telos: &ReconcileTelos) -> Result<Vec<PreparedBlock>, TemperError> {
    let mut blocks = Vec::with_capacity(telos.blocks.len());
    for (seq, b) in telos.blocks.iter().enumerate() {
        let chunks = unpack_incoming_chunks(&b.chunks_packed)?;
        blocks.push(temper_substrate::content::prepare_block_from_chunks(
            seq as i32,
            Some(&b.role),
            chunks,
        ));
    }
    Ok(blocks)
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

/// Parameters for [`validate_managed_meta_pipeline`] — the shared create/update validation gate.
struct ManagedValidationParams<'a> {
    /// The caller-supplied managed_meta as a JSON value (pre-strip).
    raw_managed: serde_json::Value,
    doc_type: &'a str,
    title: &'a str,
    /// Slug seeded into the schema validator (create: the raw command slug; update: the effective slug).
    /// Identity keys (`temper-title`/`temper-slug`/…) are injected into the validation document by
    /// `assemble_frontmatter_document` from these typed params — the pipeline no longer pre-injects them.
    validator_slug: &'a str,
    context_name: &'a str,
    /// Validation-document id + created stamp — NOT persisted from here (the substrate mints the
    /// real resource id); they only seed the validation document.
    id: uuid::Uuid,
    created: chrono::DateTime<Utc>,
}

/// The shared managed-meta validation pipeline used by BOTH `create_resource` and `update_resource`:
/// strip caller-echoed system keys → apply doc-type defaults → validate against the doc-type schema
/// (PROPAGATING the typed `BadRequest`, never swallowing it). Identity keys are injected into the
/// validation document by `validate_managed_meta` → `assemble_frontmatter_document` from the typed
/// title/slug params, so the pipeline no longer pre-injects them (managed_meta is Property-only).
///
/// Returns the assembled (defaulted) managed-meta value: create writes it as properties (identity keys
/// are Die-fated and dropped by `properties_from_meta` regardless); update validates with it but
/// persists only the raw caller keys (a partial PATCH), so it discards the return.
fn validate_managed_meta_pipeline(
    params: ManagedValidationParams<'_>,
) -> Result<serde_json::Value, TemperError> {
    let mut managed = temper_workflow::operations::strip_system_managed_fields(params.raw_managed);
    temper_workflow::operations::apply_defaults_value(params.doc_type, &mut managed);
    let validate_params = temper_workflow::operations::ValidateManagedMetaParams {
        id: ResourceId::from(params.id),
        created: params.created,
        doc_type: params.doc_type,
        managed_meta: Some(&managed),
        slug: params.validator_slug,
        title: params.title,
        context_name: params.context_name,
    };
    temper_workflow::operations::validate_managed_meta(&validate_params)?;
    Ok(managed)
}

/// Maps the substrate readback (`readback::resource_row`) to the native `ResourceRow` — real
/// timestamps (event-sourced from `kb_events.occurred_at`), name-only doc type, no fabrication.
/// Shared by `show_resource` and the read selector arms (`list_select`, `show_select`,
/// `search_select`). The §7-dissolved fields (`kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`)
/// are absent; `doc_type_name` is authoritative.
pub(crate) async fn native_resource_row(
    pool: &PgPool,
    principal: ProfileId,
    new_id: ResourceId,
) -> Result<ResourceRow, TemperError> {
    let p = readback::resource_row(pool, principal, new_id)
        .await
        .map_err(map_readback_err)?;
    Ok(ResourceRow {
        id: p.re_minted_id,
        kb_context_id: p.re_minted_context_id,
        origin_uri: p.origin_uri,
        title: p.title,
        originator_profile_id: p.originator_profile_id,
        owner_profile_id: p.owner_profile_id,
        is_active: p.is_active,
        created: p.created,
        updated: p.updated,
        context_name: p.context_name,
        doc_type_name: p.doc_type_name,
        owner_handle: p.owner_handle,
        context_slug: p.context_slug,
        context_owner_ref: p.context_owner_ref,
        cogmap_id: p.cogmap_id,
        cogmap_name: p.cogmap_name,
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
/// behalf (`owner`), under which event `emitter`, and the run's authored-act context (`act` —
/// `invocation: Some(inv)` for the reconcile's own minted envelope + the caller's `authorship`). Every
/// mutation a phase fires stamps `act`, so the whole reconcile is queryable by its `invocation_id`.
/// Bundled so each phase helper takes one context argument instead of threading the ids — and to stay
/// under the params-struct threshold. `Clone` (not `Copy`): `act` carries owned authorship.
#[derive(Clone, Debug)]
struct ReconcileCtx {
    cogmap: CogmapId,
    owner: ProfileId,
    emitter: EntityId,
    act: EventContext,
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

    /// Auth-before-writes gate for the invocation envelope: the acting profile (`self.profile_id`)
    /// must be able to READ the originating cognitive map. Calls the canonical visibility predicate
    /// `anchor_readable_by_profile(profile, 'kb_cogmaps', cogmap_id)` directly (the retired
    /// `cogmap_readable_by_profile` is reached only via this anchor arm; `require_cogmap_write_admin`
    /// is the wrong gate — it is a structural L0/root-team gate that admits ordinary cogmaps). Deny →
    /// `Forbidden` (403). The substrate's `invocation_open` enforces the parent→originating delegation
    /// gate itself, so this is ONLY the acting-profile-can-access-originating check.
    async fn check_can_read_cogmap(&self, cogmap_id: uuid::Uuid) -> Result<(), TemperError> {
        let can: Option<bool> = sqlx::query_scalar!(
            "SELECT anchor_readable_by_profile($1, 'kb_cogmaps', $2)",
            *self.profile_id,
            cogmap_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(api_err)?;
        if can.unwrap_or(false) {
            Ok(())
        } else {
            Err(TemperError::Forbidden)
        }
    }

    /// Auth-before-writes gate for authoring INTO a cognitive map: the acting profile must hold an
    /// explicit write grant on the map (`cogmap_authorable_by_profile` =
    /// `profile_explicit_grant(...,'write','kb_cogmaps',...)` — cogmaps have no owner, so authority is
    /// wholly explicit). Two callers: the backend-side create-into-cogmap gate (F1 — belt-and-suspenders
    /// for the same predicate the surfaces pre-check, so the shared write path denies even a caller that
    /// skipped the surface) and the self-attributed `invocation_open` gate (F2). Deny → `Forbidden` (403).
    async fn check_cogmap_authorable(&self, cogmap_id: uuid::Uuid) -> Result<(), TemperError> {
        let can: Option<bool> = sqlx::query_scalar!(
            "SELECT cogmap_authorable_by_profile($1, $2)",
            *self.profile_id,
            cogmap_id,
        )
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

    /// Assert an edge from `edge.src` to `edge.tgt`, homing it on the SOURCE resource's
    /// anchor — a context for ordinary resources, or the cognitive map itself for
    /// cogmap-homed nodes. Reads the source's home WITHOUT assuming a context (an
    /// `anchor_table='kb_contexts'` filter returns zero rows for cogmap-homed sources).
    ///
    /// Auth on the source is the CALLER's responsibility — this helper does NOT gate.
    /// Callers already hold it: `assert_relationship` runs its source modify-check first;
    /// the create/update goal projection owns the resource under mutation. Shared so the
    /// home-detect + kernel-vs-context branch lives in exactly one place.
    async fn assert_edge_from_source_home(
        &self,
        edge: SourceHomedEdge<'_>,
        act_ctx: EventContext,
    ) -> Result<EdgeId, TemperError> {
        let (home_id, home_table): (uuid::Uuid, String) = sqlx::query_as(
            "SELECT anchor_id, anchor_table FROM kb_resource_homes \
             WHERE resource_id=$1 AND anchor_table IN ('kb_contexts', 'kb_cogmaps') \
             LIMIT 1",
        )
        .bind(edge.src)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

        let label = (!edge.label.is_empty()).then_some(edge.label);
        let src = ResourceId::from(edge.src);
        let tgt = ResourceId::from(edge.tgt);
        let kind = map_edge_kind(edge.edge_kind);
        let polarity = map_polarity(edge.polarity);
        let weight = edge.weight;
        let emitter = edge.emitter;
        let edge = if home_table == "kb_cogmaps" {
            writes::assert_kernel_edge_with(
                &self.pool,
                writes::KernelEdgeParams {
                    cogmap: CogmapId::from(home_id),
                    src,
                    tgt,
                    kind,
                    polarity,
                    label,
                    weight,
                    emitter,
                },
                act_ctx,
            )
            .await
            .map_err(api_err)?
        } else {
            writes::assert_relationship_with(
                &self.pool,
                writes::AssertParams {
                    src,
                    tgt,
                    kind,
                    polarity,
                    label,
                    weight,
                    home: ContextId::from(home_id),
                    emitter,
                },
                act_ctx,
            )
            .await
            .map_err(api_err)?
        };
        Ok(EdgeId::from(edge.uuid()))
    }

    /// Fold the source resource's outgoing `advances`→goal edges (at most one under the
    /// one-goal-per-resource model; folds all defensively). Used by the update path to
    /// retract (`GoalPatch::Clear`) or replace (`GoalPatch::Set`) a goal.
    ///
    /// The `doc_type='goal'` join is load-bearing: a single resource may hold BOTH a
    /// session→task `advances` edge and a →goal `advances` edge (same kind+label), and
    /// only the latter must be folded — so the target's doc-type property gates it.
    async fn fold_goal_edges(
        &self,
        src_next: uuid::Uuid,
        emitter: EntityId,
        act_ctx: &EventContext,
    ) -> Result<(), TemperError> {
        let edge_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
            "SELECT e.id FROM kb_edges e \
             JOIN kb_properties p \
               ON p.owner_table = 'kb_resources' AND p.owner_id = e.target_id \
              AND p.property_key = 'doc_type' AND NOT p.is_folded \
             WHERE e.source_table = 'kb_resources' AND e.source_id = $1 \
               AND e.target_table = 'kb_resources' \
               AND e.edge_kind = 'leads_to' AND e.label = $2 \
               AND p.property_value #>> '{}' = 'goal' \
               AND NOT e.is_folded",
        )
        .bind(src_next)
        .bind(GOAL_EDGE_LABEL)
        .fetch_all(&self.pool)
        .await
        .map_err(api_err)?;

        for eid in edge_ids {
            writes::fold_relationship_with(
                &self.pool,
                EdgeId::from(eid),
                Some("goal reassigned"),
                emitter,
                act_ctx.clone(),
            )
            .await
            .map_err(api_err)?;
        }
        Ok(())
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
        cogmap: CogmapId,
        request: &ReconcileCogmapRequest,
        owner: ProfileId,
        emitter: EntityId,
        run_ctx: EventContext,
    ) -> Result<ReconcileOutcome, TemperError> {
        let ctx = ReconcileCtx {
            cogmap,
            owner,
            emitter,
            act: run_ctx,
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
        Self::apply_resource_phase(&mut *conn, request, &live_by_id, ctx.clone(), &mut outcome)
            .await?;
        Self::apply_edge_phase(&mut *conn, request, ctx.clone()).await?;
        Self::apply_tombstone_phase(&mut *conn, request, &live_by_id, ctx.clone(), &mut outcome)
            .await?;
        Self::apply_telos_phase(&mut *conn, request, ctx, &mut outcome).await?;

        Ok(outcome)
    }

    /// Read the live `provenance: kernel` slice and index it by the STABLE resource id — the diff
    /// key. (`origin_uri` is loose, non-unique attribution and is NEVER a key.) Owns the rows so the
    /// index outlives the read without borrowing the source vec.
    async fn read_kernel_index(
        conn: &mut sqlx::PgConnection,
        cogmap_uuid: uuid::Uuid,
    ) -> Result<std::collections::HashMap<uuid::Uuid, readback::KernelSliceRow>, TemperError> {
        let live = readback::kernel_slice(&mut *conn, CogmapId::from(cogmap_uuid))
            .await
            .map_err(api_err)?;
        // The reconcile diff keys on the bare uuid (entry ids arrive bare from the wire), so index on
        // the inner uuid of the now-typed `resource_id`.
        Ok(live
            .into_iter()
            .map(|r| (r.resource_id.uuid(), r))
            .collect())
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
                        ctx.act.clone(),
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
                        ctx.act.clone(),
                    )
                    .await
                    .map_err(api_err)?;

                    // Clustering facets (e.g. `layer`) — strip any stray `provenance` (stamped above,
                    // never clustered). Skip the write entirely when there's nothing to cluster.
                    let facets = strip_provenance_facet(&entry.facets);
                    if facet_is_nonempty(&facets) {
                        writes::set_facet_in_tx(
                            &mut *conn,
                            rid,
                            &facets,
                            1.0,
                            ctx.emitter,
                            ctx.act.clone(),
                        )
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
                            resource: row.resource_id,
                            // `Some("")` requests a body re-block (the re-block fires iff body is
                            // `Some`); the content comes from `chunks` (always `Some` here), so the
                            // empty string is never embedded — see the CREATE arm's note.
                            body: Some(""),
                            title: None,
                            origin_uri: None,
                            properties: &[],
                            chunks: Some(incoming_chunks),
                            // Corpus reconcile carries no provenance attribution (not a distillation).
                            sources: Vec::new(),
                            // Corpus reconcile revises the resource's sole body block (never addresses one).
                            content_block: None,
                            rehome_to: None,
                            emitter: ctx.emitter,
                        },
                        ctx.act.clone(),
                        // Corpus reconcile always carries precomputed chunks (bring-your-own vectors),
                        // so there is nothing to defer — inline.
                        false,
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
                let present = readback::find_edge(
                    &mut *conn,
                    ResourceId::from(src),
                    ResourceId::from(tgt),
                    &kind,
                    Some(&e.polarity),
                )
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
                        src: ResourceId::from(src),
                        tgt: ResourceId::from(tgt),
                        kind,
                        polarity,
                        label: e.label.as_deref(),
                        weight: e.weight,
                        emitter: ctx.emitter,
                    },
                    ctx.act.clone(),
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
                    row.resource_id,
                    ctx.emitter,
                    ctx.act.clone(),
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
            let edge_id = readback::find_edge(
                &mut *conn,
                ResourceId::from(t.from),
                ResourceId::from(t.to),
                &kind,
                None,
            )
            .await
            .map_err(api_err)?;
            if let Some(edge_id) = edge_id {
                writes::fold_relationship_in_tx(
                    &mut *conn,
                    edge_id,
                    Some("reconcile fold"),
                    ctx.emitter,
                    // Correlated to the reconcile run (its minted invocation + the caller's authorship),
                    // like every other act this loop fires.
                    ctx.act.clone(),
                )
                .await
                .map_err(api_err)?;
                outcome.folded += 1;
            }
        }
        Ok(())
    }

    /// PHASE 4 — the telos charter (distinct grain from the kernel slice). Diff on the telos's two-level
    /// body merkle; fire `cogmap_charter_set` only on change; record `outcome.charter`. A request with no
    /// `telos:` leaves `charter = Absent`.
    async fn apply_telos_phase(
        conn: &mut sqlx::PgConnection,
        request: &ReconcileCogmapRequest,
        ctx: ReconcileCtx,
        outcome: &mut ReconcileOutcome,
    ) -> Result<(), TemperError> {
        let Some(telos) = &request.telos else {
            return Ok(());
        }; // charter stays Absent

        // Unpack + prepare each role-tagged block (client-embedded chunks, verbatim).
        let blocks = prepare_telos_blocks(telos)?;

        // Incoming resource merkle (two-level), compared to the live telos body_hash — the diff key.
        let per_block: Vec<Vec<String>> = blocks
            .iter()
            .map(|blk| blk.chunks.iter().map(|c| c.content_hash.clone()).collect())
            .collect();
        let incoming = temper_substrate::content::body_hash_from_block_chunk_hashes(&per_block);

        let live = readback::telos_charter_state(&mut *conn, ctx.cogmap)
            .await
            .map_err(api_err)?;

        if live.body_hash.as_deref() == Some(incoming.as_str()) {
            outcome.charter = CharterDisposition::Unchanged;
            return Ok(());
        }

        writes::set_charter_in_tx(
            &mut *conn,
            ctx.cogmap,
            &blocks,
            ctx.emitter,
            ctx.act.clone(),
        )
        .await
        .map_err(api_err)?;

        let empty = temper_substrate::content::body_hash_for_body("");
        // `None` means the telos row exists but has no body yet (genesis / pre-charter state):
        // also counts as first delivery, not a revision.
        outcome.charter =
            if live.body_hash.is_none() || live.body_hash.as_deref() == Some(empty.as_str()) {
                CharterDisposition::Created
            } else {
                CharterDisposition::Updated
            };
        Ok(())
    }
}

impl DbBackend {
    /// Per-act correlation-integrity gate. When an authored act carries an `invocation_id`, the caller
    /// must be able to read the invocation's originating cogmap (absent OR unreadable → uniform 404, no
    /// existence oracle — matching the `invocation_show`/`close_invocation` deny→NotFound contract), and
    /// the run must still be `open` (a closed run is a 409 — you cannot stamp new acts onto a terminal
    /// envelope).
    ///
    /// This is ADDITIVE to the act's own write authz (`can_modify`, context-owner resolution) — it never
    /// authorizes the write, it only guards the *correlation claim*. An act with no invocation skips it
    /// entirely (a one-off attributed act, or a human at the same tools, is fully valid).
    async fn check_act_invocation(
        &self,
        invocation: Option<InvocationId>,
    ) -> Result<(), TemperError> {
        let Some(inv) = invocation else {
            return Ok(());
        };
        let row = sqlx::query!(
            "SELECT status \
               FROM kb_invocations \
              WHERE id = $1 \
                AND anchor_readable_by_profile($2, 'kb_cogmaps', originating_cogmap_id)",
            inv.uuid(),
            *self.profile_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        .ok_or_else(|| TemperError::NotFound(format!("invocation {} not found", inv.uuid())))?;
        if row.status != "open" {
            return Err(TemperError::Conflict(format!(
                "invocation {} is '{}' — cannot stamp an act onto a non-open run",
                inv.uuid(),
                row.status
            )));
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
        // Resolve the caller's synthesized identity (natural-key).
        // `cmd.home` is a pre-resolved HomeAnchor — surfaces parse+resolve the ref
        // before building the command, so no `writes::resolve_context` call is needed here.
        let prod_profile: uuid::Uuid = *self.profile_id;
        let owner = writes::resolve_profile(&self.pool, prod_profile)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        // Correlation-integrity gate for any claimed invocation — additive to the create authz above,
        // before any mutation (auth-before-write). No-op when the act carries no invocation.
        self.check_act_invocation(cmd.act.invocation).await?;
        // F1 — backend-side create-into-cogmap gate (auth before writes). The surfaces (mcp create
        // tool, api ingest) pre-check `cogmap_authorable_by_profile` too, but the shared write path
        // must not trust them: a gate that lives only on the surfaces is one new caller away from a
        // silent bypass (the SAML `is_active` failure mode `docs/auth` exists to prevent). Cogmap-only
        // — create-into-context is not gated here (a deliberate scope guard; see the cascade spec).
        if let HomeAnchor::Cogmap(m) = &cmd.home {
            self.check_cogmap_authorable(uuid::Uuid::from(*m)).await?;
        }
        // Map the command's HomeAnchor to the substrate's AnchorRef so CreateParams.home
        // accepts either a context or a cognitive map without further branching downstream.
        let home = match cmd.home {
            HomeAnchor::Context(c) => AnchorRef::context(c),
            HomeAnchor::Cogmap(m) => AnchorRef::cogmap(m),
        };

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

        // Create-time guards (WS6 collapse Task F): the shared strip → defaults → identity-keys →
        // validate pipeline (see `validate_managed_meta_pipeline`), the same one the legacy
        // `ingest_service::ingest` ran. A fresh canonical id + `now()` seed the validation document
        // (not persisted — the substrate mints the real id in `writes::create_resource`). An empty
        // slug removes `temper-slug` (mirrors ingest's `injected_slug`).
        let managed = validate_managed_meta_pipeline(ManagedValidationParams {
            raw_managed: serde_json::to_value(&cmd.managed_meta)
                .map_err(|e| TemperError::Api(e.to_string()))?,
            doc_type: &cmd.doctype,
            title: &cmd.title,
            validator_slug: &cmd.slug,
            // Validation-only placeholder. For a cogmap home this is the raw cogmap UUID, not a
            // context name — `context_name` is display-only in the validation document (§7-dissolving),
            // so the mislabel is intentional and never persisted as a name.
            context_name: &home.id.to_string(),
            id: uuid::Uuid::now_v7(),
            created: Utc::now(),
        })?;

        // No content-hash dedup on create. A resource's identity is its id + its position in the
        // relationship graph, NOT its body: empty/template/placeholder concepts legitimately recur
        // across contexts and cogmaps (an interstitial concept's edges are the point, the body may be
        // a generic stub), and a co-member who can read a cogmap can see its homed resources — so a
        // body-hash match against the *visible* set (the retired `find_by_body_hash`) would collapse
        // distinct resources into one (e.g. the empty-bodied L0 kernel telos). Re-creating identical
        // content yields a distinct resource by design.
        let properties = properties_from_meta(&managed, cmd.open_meta.as_ref());

        // Map the surface-supplied ActContext → substrate EventContext (identical re-exported types).
        // The authored `resource_created` act carries this authorship + invocation; the property acts
        // fired at creation stay un-stamped (out of the authored-act scope).
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        // Block-provenance: the body's declared sources, position → accretion `seq`.
        let sources = body_sources(cmd.body.as_ref());
        // Defer embedding (issue #299) only for the SERVER-COMPUTED case — no precomputed chunks and a
        // non-empty body — and only when this deployment has the drain enabled. Caller-supplied chunks
        // (CLI/API bring-your-own vectors) always persist inline; there is nothing to defer.
        let defer = incoming_chunks.is_none()
            && !body.is_empty()
            && crate::services::embed_service::async_embed_enabled();
        let params = writes::CreateParams {
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
            sources,
        };
        let new_id = if defer {
            writes::create_resource_deferred_with(&self.pool, params, act_ctx.clone()).await
        } else {
            writes::create_resource_with(&self.pool, params, act_ctx.clone()).await
        }
        .map_err(api_err)?;

        // Project the first-class goal link to a live `advances`→goal edge (issue 019f3d55). The
        // new resource is the source (owned by the caller, so the shared assert helper's "auth is
        // the caller's responsibility" contract holds); the edge homes on the resource's anchor.
        if let Some(goal) = cmd.goal {
            self.assert_edge_from_source_home(
                SourceHomedEdge {
                    src: new_id.uuid(),
                    tgt: goal.into(),
                    edge_kind: graph::EdgeKind::LeadsTo,
                    polarity: graph::Polarity::Forward,
                    label: GOAL_EDGE_LABEL,
                    weight: 1.0,
                    emitter,
                },
                act_ctx,
            )
            .await?;
        }

        if defer {
            // Enqueue the off-request embed backfill. A failed enqueue must NOT fail the create — the
            // resource is fully formed and FTS-searchable; it just stays vector-unsearchable until a
            // re-drive (a failed-job sweep / reindex) recovers it. Log and proceed.
            if let Err(e) = crate::services::workflow_job_service::enqueue_resource(
                &self.pool,
                new_id.uuid(),
                Persona::Embed.as_str(),
                DispatchType::Embed.as_str(),
            )
            .await
            {
                tracing::warn!(
                    resource_id = %new_id.uuid(),
                    error = %e,
                    "failed to enqueue embed backfill; resource is FTS-only until re-driven"
                );
            }
        }

        let row = native_resource_row(&self.pool, self.profile_id, ResourceId::from(new_id.uuid()))
            .await?;
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
        let row =
            native_resource_row(&self.pool, self.profile_id, ResourceId::from(new_id)).await?;
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
        // Correlation-integrity gate for any claimed invocation — additive to the modify authz above,
        // before any mutation. No-op when the act carries no invocation.
        self.check_act_invocation(cmd.act.invocation).await?;
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
        // Identity is first-class on the cmd: `cmd.title` updates the kb_resources.title
        // column on any PATCH, independent of whether managed_meta is present (a §7-Die key,
        // never a property).
        let title: Option<String> = cmd.title.clone();
        let mut properties: Vec<(String, serde_json::Value)> = Vec::new();
        // Validate whenever the caller touches managed_meta OR identity: a title change must
        // still satisfy the schema (e.g. non-empty `temper-title`), so a title-only PATCH runs
        // the pipeline with an empty managed value.
        if cmd.managed_meta.is_some() || cmd.title.is_some() {
            let incoming = match &cmd.managed_meta {
                Some(mm) => {
                    serde_json::to_value(mm).map_err(|e| TemperError::Api(e.to_string()))?
                }
                None => serde_json::json!({}),
            };

            // Update-time validation (mirror create's strip→defaults→validate pipeline). Update
            // carries no doc_type/context, so take the EFFECTIVE values from the current row
            // (already visibility-gated via `check_can_modify_next`).
            let current =
                native_resource_row(&self.pool, self.profile_id, ResourceId::from(new_id)).await?;
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
                .and_then(|m| m.context_to.map(|id| id.to_string()))
                .or_else(|| current.context_name.clone())
                .unwrap_or_default();
            // Effective identity seeds the validation document: `cmd.title` else the current
            // title; `cmd.slug` else the canonical slug derived from the effective title (so the
            // `temper-slug`-required schemas don't FALSE-reject a valid update).
            let effective_title = cmd.title.clone().unwrap_or_else(|| current.title.clone());
            let effective_slug = cmd
                .slug
                .clone()
                .unwrap_or_else(|| temper_workflow::operations::sluggify(&effective_title));

            // Validate via the SHARED pipeline (see `validate_managed_meta_pipeline`) — the same
            // strip → defaults → validate as create; PROPAGATE the typed BadRequest (an
            // out-of-enum value or an unknown doc_type → 400, the create contract). Identity is
            // injected into the validation document from the effective title/slug, so a partial
            // update never false-rejects. The assembled set is validation-only; update persists
            // the raw caller keys (partial PATCH), so the return is discarded.
            validate_managed_meta_pipeline(ManagedValidationParams {
                raw_managed: incoming.clone(),
                doc_type: &effective_doc_type,
                title: &effective_title,
                validator_slug: &effective_slug,
                context_name: &effective_context,
                id: new_id,
                created: current.created,
            })?;

            // Write only the caller-supplied keys (PATCH is a partial merge; `PropertySet`
            // asserts per key, so unsupplied keys are untouched — DON'T write the defaulted
            // validation set). `properties_from_meta` filters to §7-Property keys, so the
            // §7-Die identity keys + the §7-ReconcileToDocType `temper-type` never become rows.
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
            if let Some(ctx_to) = mv.context_to {
                // The ContextId was already resolved and visibility-gated at the
                // handler boundary (parse_context_ref + resolve_context_ref). Use it
                // directly; no second DB lookup needed.
                rehome_to = Some(ctx_to);
            }
        }

        // ActContext → EventContext: every sub-event of the update fan-out is correlated + authored.
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        // Defer embedding (issue #299) only when this revise re-blocks the body server-side: a
        // non-empty body change with no precomputed chunks, and the deployment drain is enabled. A
        // meta-only update (body None) re-chunks nothing, so there is nothing to defer or enqueue.
        let defer = incoming_chunks.is_none()
            && body.as_deref().is_some_and(|b| !b.is_empty())
            && crate::services::embed_service::async_embed_enabled();
        let params = writes::UpdateParams {
            resource: ResourceId::from(new_id),
            body: body.as_deref(),
            title: title.as_deref(),
            origin_uri: None,
            properties: &properties,
            chunks: incoming_chunks,
            sources: body_sources(cmd.body.as_ref()),
            content_block: cmd.body.as_ref().and_then(|b| b.content_block),
            rehome_to,
            emitter,
        };
        if defer {
            writes::update_resource_deferred_with(&self.pool, params, act_ctx.clone()).await
        } else {
            writes::update_resource_with(&self.pool, params, act_ctx.clone()).await
        }
        .map_err(api_err)?;

        // Project the goal-edge patch (issue 019f3d55). `Set` folds any existing `advances`→goal
        // edge and asserts the new one (replace-in-place); `Clear` folds without re-asserting;
        // `None` leaves the goal edge untouched. The modify-gate above is this update's own auth,
        // so the shared assert helper's "caller owns auth" contract holds.
        match cmd.goal {
            Some(GoalPatch::Set(goal)) => {
                self.fold_goal_edges(new_id, emitter, &act_ctx).await?;
                self.assert_edge_from_source_home(
                    SourceHomedEdge {
                        src: new_id,
                        tgt: goal.into(),
                        edge_kind: graph::EdgeKind::LeadsTo,
                        polarity: graph::Polarity::Forward,
                        label: GOAL_EDGE_LABEL,
                        weight: 1.0,
                        emitter,
                    },
                    act_ctx,
                )
                .await?;
            }
            Some(GoalPatch::Clear) => {
                self.fold_goal_edges(new_id, emitter, &act_ctx).await?;
            }
            None => {}
        }

        if defer {
            // Enqueue the off-request backfill for the revised chunks. Resource-id single-flight means a
            // re-enqueue over an in-flight embed dedups; the reaper covers a lost one. A failed enqueue
            // never fails the revise (the body is FTS-searchable now).
            if let Err(e) = crate::services::workflow_job_service::enqueue_resource(
                &self.pool,
                new_id,
                Persona::Embed.as_str(),
                DispatchType::Embed.as_str(),
            )
            .await
            {
                tracing::warn!(
                    resource_id = %new_id,
                    error = %e,
                    "failed to enqueue embed backfill on update; resource is FTS-only until re-driven"
                );
            }
        }

        let row =
            native_resource_row(&self.pool, self.profile_id, ResourceId::from(new_id)).await?;
        Ok(CommandOutput::new(row))
    }

    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>, TemperError> {
        // The inbound id IS the substrate resource id (no origin_uri remap).
        let new_id = uuid::Uuid::from(cmd.resource);
        // Auth before any write (WS2).
        self.check_can_modify_next(new_id).await?;
        // Correlation-integrity gate — additive to the modify authz above, before the write.
        self.check_act_invocation(cmd.act.invocation).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        writes::delete_resource_with(&self.pool, ResourceId::from(new_id), emitter, act_ctx)
            .await
            .map_err(api_err)?;
        Ok(CommandOutput::new(()))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let rows = readback::list(&self.pool, self.profile_id)
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
        let ids = readback::fts_search(&self.pool, self.profile_id, &cmd.query.query)
            .await
            .map_err(|e| TemperError::Api(e.to_string()))?;
        let mut hits = Vec::with_capacity(ids.len());
        for new_id in ids {
            let row = native_resource_row(&self.pool, self.profile_id, new_id).await?;
            let context = row.home_display().unwrap_or_default().to_owned();
            hits.push(SearchHit {
                summary: ResourceSummary {
                    // slug is §7-dissolved; the summary uses origin_uri as the stable handle.
                    slug: row.origin_uri,
                    doctype: row.doc_type_name,
                    context,
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
    ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
        // Source and target ids ARE the substrate resource ids (synthesis carries resource ids verbatim)
        // — used directly, no origin_uri remap (the prior bimap collapsed empty-origin_uri resources
        // onto one arbitrary id).
        let src_next = uuid::Uuid::from(cmd.source);
        // Auth before any write (WS2): edge mutations gate on the SOURCE resource (production's
        // "Cannot modify source resource"). Gate before resolving the target / writing the edge.
        self.check_can_modify_next(src_next).await?;
        // Correlation-integrity gate — additive to the modify-source authz above, never a substitute.
        self.check_act_invocation(cmd.act.invocation).await?;

        let tgt_next = uuid::Uuid::from(cmd.target);

        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        // Home-detect + kernel-vs-context branch is shared with the create/update goal-edge
        // projection via `assert_edge_from_source_home`. The source modify-gate above is this
        // command's own auth; the helper does not re-gate.
        let edge = self
            .assert_edge_from_source_home(
                SourceHomedEdge {
                    src: src_next,
                    tgt: tgt_next,
                    edge_kind: cmd.edge_kind,
                    polarity: cmd.polarity,
                    label: &cmd.label,
                    weight: cmd.weight,
                    emitter,
                },
                act_ctx,
            )
            .await?;
        Ok(CommandOutput::new(edge))
    }

    async fn retype_relationship(
        &self,
        cmd: RetypeRelationship,
    ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
        // The edge handle on the substrate backend IS the substrate edge id (returned by assert).
        let handle = uuid::Uuid::from(cmd.edge_handle);
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(handle).await?;
        self.check_can_modify_next(src).await?;
        // Correlation-integrity gate — additive to the modify authz above, before the write.
        self.check_act_invocation(cmd.act.invocation).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        writes::retype_relationship_with(
            &self.pool,
            EdgeId::from(handle),
            map_edge_kind(cmd.edge_kind),
            map_polarity(cmd.polarity),
            emitter,
            act_ctx,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    async fn reweight_relationship(
        &self,
        cmd: ReweightRelationship,
    ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
        let handle = uuid::Uuid::from(cmd.edge_handle);
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(handle).await?;
        self.check_can_modify_next(src).await?;
        // Correlation-integrity gate — additive to the modify authz above, before the write.
        self.check_act_invocation(cmd.act.invocation).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        writes::reweight_relationship_with(
            &self.pool,
            EdgeId::from(handle),
            cmd.weight,
            emitter,
            act_ctx,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    async fn fold_relationship(
        &self,
        cmd: FoldRelationship,
    ) -> Result<CommandOutput<temper_core::types::ids::EdgeId>, TemperError> {
        let handle = uuid::Uuid::from(cmd.edge_handle);
        // Auth before any write (WS2): gate on the edge's source resource.
        let src = self.edge_source_resource(handle).await?;
        self.check_can_modify_next(src).await?;
        // Correlation-integrity gate — additive to the modify-source authz above.
        self.check_act_invocation(cmd.act.invocation).await?;
        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        writes::fold_relationship_with(
            &self.pool,
            EdgeId::from(handle),
            cmd.reason.as_deref(),
            emitter,
            act_ctx,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(cmd.edge_handle))
    }

    /// Upserts the clustering `facet` property (`kb_properties`) on a resource — one row holding the
    /// whole `values` object. Mirrors `assert_relationship`/`fold_relationship`'s auth + owner/emitter
    /// resolution, gated on the TARGET resource directly (facets have no source/target split).
    async fn set_facet(&self, cmd: SetFacet) -> Result<CommandOutput<PropertyId>, TemperError> {
        let resource_next = uuid::Uuid::from(cmd.resource);
        // Auth before any write (WS2): gate on the resource the facet is being set on.
        self.check_can_modify_next(resource_next).await?;
        // Correlation-integrity gate — additive to the modify authz above, before the write.
        self.check_act_invocation(cmd.act.invocation).await?;

        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;
        let act_ctx = EventContext {
            invocation: cmd.act.invocation,
            authorship: cmd.act.authorship,
        };
        let property_id = writes::set_facet_with(
            &self.pool,
            cmd.resource,
            &cmd.values,
            cmd.weight,
            emitter,
            act_ctx,
        )
        .await
        .map_err(map_facet_write_err)?;
        Ok(CommandOutput::new(property_id))
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
        let cogmap_uuid = uuid::Uuid::from(cmd.cogmap_id);
        let cogmap = CogmapId::from(cogmap_uuid);

        // The system actor: every kernel mutation fires under (owner = system profile, emitter = system
        // entity) — the L0 birth migration's actor.
        let (owner, emitter) = readback::system_actor(&self.pool).await.map_err(api_err)?;

        // PRE-FLIGHT (FIX #3) — fail fast + loud on an unresolved edge target, BEFORE opening the
        // transaction, so a bad manifest writes NOTHING. A quick read on the pool; the authoritative
        // in-tx read still happens inside `reconcile_apply`.
        self.preflight_validate_edge_targets(cogmap_uuid, &cmd.request)
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

        // The run's authored-act context: every mutation reconcile_apply fires correlates to THIS
        // reconcile's own minted envelope (`inv`) and carries the caller's authorship. Any
        // caller-supplied `cmd.act.invocation` is ignored — the reconcile owns its envelope.
        let run_ctx = EventContext {
            invocation: Some(inv),
            authorship: cmd.act.authorship.clone(),
        };

        // APPLY on the same connection. On ANY error the `?` returns here with `tx` dropped → full
        // rollback (mutations + the envelope open). No Failed-close needed.
        let outcome = self
            .reconcile_apply(&mut tx, cogmap, &cmd.request, owner, emitter, run_ctx)
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

    /// Genesis (create) a new cognitive map (cogmap + telos charter resource) from a manifest, under the
    /// system actor, as a SINGLE `SERIALIZABLE` transaction. Idempotent at a given id: re-genesis
    /// returns the existing identity with `created: false` and fires NOTHING. Create is open to any
    /// authenticated profile (no surface admin gate). Two guards live HERE: a caller-supplied
    /// `cogmap_id`/`telos_resource_id` is honored only for a system-admin (else server-minted — a
    /// non-admin can never place a map at a chosen, e.g. reserved, id), and the creator is granted
    /// read+write+grant on the new map (below).
    async fn create_cognitive_map(
        &self,
        cmd: CreateCognitiveMap,
    ) -> Result<CommandOutput<CreateCogmapOutcome>, TemperError> {
        // Reserved-id hardening: honor a caller-supplied id ONLY for a system-admin. A non-admin's ids
        // are ignored and the server mints fresh uuidv7s, so a non-admin can never place a map at a
        // chosen (e.g. reserved L0/system) id. Explicit-id genesis stays operator work. Resolving the
        // identity HERE (not deferring to the firing arm's `unwrap_or_else`) lets the existence
        // pre-check key on the realized id and lets the outcome echo a stable id even on the mint path.
        let caller_is_admin =
            crate::services::access_service::is_system_admin(&self.pool, self.profile_id)
                .await
                .map_err(|e| TemperError::Api(e.to_string()))?;
        let requested_cogmap_id = if caller_is_admin {
            cmd.request.cogmap_id
        } else {
            None
        };
        let requested_telos_id = if caller_is_admin {
            cmd.request.telos_resource_id
        } else {
            None
        };
        let cogmap_id = requested_cogmap_id
            .map(CogmapId::from)
            .unwrap_or_else(|| CogmapId::from(uuid::Uuid::now_v7()));
        let telos_resource_id = requested_telos_id
            .map(ResourceId::from)
            .unwrap_or_else(|| ResourceId::from(uuid::Uuid::now_v7()));
        let cogmap_uuid = uuid::Uuid::from(cogmap_id);

        // IDEMPOTENT NO-OP (FIX #3): re-genesis at an existing id is a no-op. `_project_cogmap_seeded`
        // does plain INSERTs (no ON CONFLICT), so a second genesis at a live id would duplicate-key.
        // Pre-check existence on the pool BEFORE opening any transaction: if the map exists, return its
        // STORED telos id with `created: false` and open NO envelope / fire NOTHING (no duplicate
        // kb_events row). The `kb_cogmaps` PK is the concurrency backstop — a genesis race that slips
        // past this read maps to a duplicate-key/serialization error → `Conflict` at commit.
        if let Some(existing_telos) = sqlx::query_scalar!(
            "SELECT telos_resource_id FROM kb_cogmaps WHERE id = $1",
            cogmap_uuid,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        {
            return Ok(CommandOutput::new(CreateCogmapOutcome {
                cogmap_id: cogmap_uuid,
                telos_resource_id: existing_telos,
                created: false,
            }));
        }

        // The system actor: genesis fires under (owner = system profile, emitter = system entity) — the
        // L0 birth migration's actor (mirrors reconcile).
        let (owner, emitter) = readback::system_actor(&self.pool).await.map_err(api_err)?;

        // Charter blocks: client-embedded chunks carried verbatim (NO server ONNX). Absent telos ⇒ an
        // empty charter (the map is born empty, deliverable later via reconcile's `CharterSet`).
        let blocks = match &cmd.request.telos {
            Some(telos) => prepare_telos_blocks(telos)?,
            None => Vec::new(),
        };

        // ONE SERIALIZABLE transaction for the whole run (mirrors reconcile). `SET TRANSACTION ISOLATION
        // LEVEL SERIALIZABLE` must be the first statement after BEGIN.
        let mut tx = self.pool.begin().await.map_err(api_err)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(api_err)?;

        // FIRE GENESIS FIRST — unlike reconcile (which reconciles an EXISTING map), genesis CREATES the
        // cogmap, and `kb_invocations.originating_cogmap_id` FK-references `kb_cogmaps(id)` (the
        // `delegated_launch` projection RAISEs if the originating cogmap is absent). So the cogmap must
        // exist before the envelope can reference it. The `cogmap_seeded` event is its own correlation
        // root (matching reconcile's mutations, which the envelope likewise does not stamp).
        let (born_cogmap, born_telos) = fire_with(
            &mut tx,
            SeedAction::CogmapGenesis {
                name: &cmd.request.name,
                telos_title: &cmd.request.telos_title,
                charter: &blocks,
                cogmap_id: Some(cogmap_id),
                telos_resource_id: Some(telos_resource_id),
                owner,
                emitter,
            },
            EventContext::default(),
        )
        .await
        .map_err(api_err)?
        .cogmap_genesis()
        .map_err(api_err)?;

        let outcome = CreateCogmapOutcome {
            cogmap_id: uuid::Uuid::from(born_cogmap),
            telos_resource_id: uuid::Uuid::from(born_telos),
            created: true,
        };

        // OPEN the `admin_genesis` envelope on the now-existing cogmap, then CLOSE it `Completed` — both
        // in the same transaction, bracketing the genesis the way `admin_reconcile` brackets a reconcile
        // run. On ANY error before commit the `?` drops `tx` → full rollback (the genesis AND the open).
        let inv = writes::open_invocation_in_tx(
            &mut tx,
            writes::OpenParams {
                trigger_kind: "admin_genesis".to_string(),
                originating: born_cogmap,
                parent: None,
                scoped_entity: emitter,
                emitter,
            },
        )
        .await
        .map_err(api_err)?;

        let outcome_json =
            serde_json::to_value(&outcome).map_err(|e| TemperError::Api(e.to_string()))?;
        writes::close_invocation_in_tx(
            &mut tx,
            inv,
            born_cogmap,
            temper_substrate::payloads::Disposition::Completed,
            outcome_json,
            emitter,
        )
        .await
        .map_err(api_err)?;

        // Creator bootstrap grant (access-capability arc, D3b §3.B): the INVOKING admin
        // (`self.profile_id`) — NOT the system actor genesis fires under — gets read+write+grant on
        // the map they just created, a self-grant admin event. Cogmaps have no ownership floor, so
        // without this the creator could never author or add a co-author to their own (still-unbound)
        // map once the Q-A tightening lands. Only the create path reaches here (the re-genesis no-op
        // returned earlier); `ON CONFLICT DO NOTHING` guards a retried create.
        let creator = uuid::Uuid::from(self.profile_id);
        sqlx::query!(
            r#"INSERT INTO kb_access_grants
                   (subject_table, subject_id, principal_table, principal_id,
                    can_read, can_write, can_grant, granted_by_profile_id)
               VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, true, $2)
               ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING"#,
            uuid::Uuid::from(born_cogmap),
            creator,
        )
        .execute(&mut *tx)
        .await
        .map_err(api_err)?;

        // COMMIT — serialization failure (40001) → `Conflict` (a genesis race), any other DB error → 500.
        match tx.commit().await {
            Ok(()) => Ok(CommandOutput::new(outcome)),
            Err(e) => Err(map_commit_err(e)),
        }
    }

    /// Open an agent-invocation envelope, returning the server-minted invocation id. AUTH BEFORE
    /// WRITE, split by delegation (F2):
    /// * **self-attributed** (`parent_cogmap` is `None`): the acting profile must be able to AUTHOR the
    ///   originating cogmap (`check_cogmap_authorable`, an explicit write grant). Claiming a slot on a
    ///   map's accountability ledger under one's own name is an authoring act — read-to-open would let a
    ///   mere reader post self-attributed (inert) envelopes as ledger noise. In production the only
    ///   self-attributed opener is the steward, which holds write on its map, so this regresses no real
    ///   caller.
    /// * **delegated** (`parent_cogmap` is `Some`): the READ gate (`check_can_read_cogmap`) suffices; the
    ///   substrate's `invocation_open` enforces the parent→originating delegation lineage internally, so
    ///   a parent that authored may delegate an open the sub-agent principal need not itself hold write for.
    ///
    /// Deny → 403, before any `writes::` call. The id is minted by `writes::open_invocation`
    /// (server-mint v1, never caller-supplied).
    async fn open_invocation(
        &self,
        cmd: OpenInvocation,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        // Auth before any write: self-attributed opens require write on the originating cogmap;
        // delegated opens require only read (delegation lineage is checked in the substrate).
        let originating = uuid::Uuid::from(cmd.originating_cogmap);
        if cmd.parent_cogmap.is_none() {
            self.check_cogmap_authorable(originating).await?;
        } else {
            self.check_can_read_cogmap(originating).await?;
        }

        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;

        let invocation = writes::open_invocation(
            &self.pool,
            writes::OpenParams {
                trigger_kind: cmd.trigger_kind,
                originating: cmd.originating_cogmap,
                parent: cmd.parent_cogmap,
                scoped_entity: emitter,
                emitter,
            },
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(invocation.uuid()))
    }

    /// Close an agent-invocation envelope with a terminal disposition + opaque outcome. ONE gated
    /// lookup does auth + existence + terminal-state in a single round-trip: the row is returned only
    /// when the acting profile can read the originating cogmap, so absent and unreadable collapse to a
    /// uniform 404 (no existence oracle — matching the `invocation_show` deny→None contract). Close is
    /// a one-shot terminal transition; a non-open envelope is a 409 (append-only — a re-close would
    /// append a second `invocation_closed` event and overwrite the terminal record). All before any
    /// `writes::` call.
    async fn close_invocation(
        &self,
        cmd: CloseInvocation,
    ) -> Result<CommandOutput<()>, TemperError> {
        // Auth + existence in one query: the gate is in the WHERE, so a row comes back only for a
        // readable invocation. Absent OR unreadable → no row → uniform NotFound (404), never 403
        // (which would confirm the id exists). The `status` rides along for the terminal guard.
        let row = sqlx::query!(
            "SELECT originating_cogmap_id, status \
               FROM kb_invocations \
              WHERE id = $1 \
                AND anchor_readable_by_profile($2, 'kb_cogmaps', originating_cogmap_id)",
            cmd.invocation,
            *self.profile_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        .ok_or_else(|| TemperError::NotFound(format!("invocation {} not found", cmd.invocation)))?;

        // Append-only: close is a one-shot terminal transition. Re-closing a completed/failed/abandoned
        // envelope would append a second close event and overwrite its terminal record — reject it.
        if row.status != "open" {
            return Err(TemperError::Conflict(format!(
                "invocation {} is already '{}' — close is a one-shot terminal transition",
                cmd.invocation, row.status
            )));
        }
        let originating = row.originating_cogmap_id;

        let owner = writes::resolve_profile(&self.pool, *self.profile_id)
            .await
            .map_err(api_err)?;
        let emitter = writes::resolve_emitter(&self.pool, owner, surface_marker(cmd.origin))
            .await
            .map_err(api_err)?;

        writes::close_invocation(
            &self.pool,
            InvocationId::from(cmd.invocation),
            CogmapId::from(originating),
            map_disposition(cmd.disposition),
            cmd.outcome,
            emitter,
        )
        .await
        .map_err(api_err)?;
        Ok(CommandOutput::new(()))
    }

    /// Advance a cogmap's steward ingest watermark (T4a). AUTH BEFORE WRITE: one gated lookup does
    /// existence + read-visibility (absent/unreadable → uniform 404, no existence oracle) and rides
    /// the cogmap-write capability along; a readable-but-not-writable cogmap → 403. The target event
    /// must exist (the FK enforces it; a clean 404 beats a raw FK violation). The advance is a direct
    /// UPDATE of operational cursor state — NOT an authored cognitive act — so it fires no event; when
    /// T5 wires steward-run-completion it will advance the watermark as part of the invocation-close
    /// event instead of calling this bare setter.
    async fn advance_steward_watermark(
        &self,
        cmd: AdvanceStewardWatermark,
    ) -> Result<CommandOutput<uuid::Uuid>, TemperError> {
        let can_write: bool = sqlx::query_scalar!(
            r#"
            SELECT cogmap_authorable_by_profile($2, $1) AS "can_write!"
              FROM kb_cogmaps
             WHERE id = $1
               AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
            "#,
            *cmd.cogmap,
            *self.profile_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        .ok_or_else(|| {
            TemperError::NotFound(format!("cognitive map {} not found", cmd.cogmap.uuid()))
        })?;
        if !can_write {
            return Err(TemperError::Forbidden);
        }

        let event_exists: bool = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM kb_events WHERE id = $1) AS "exists!""#,
            cmd.event_id,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(api_err)?;
        if !event_exists {
            return Err(TemperError::NotFound(format!(
                "event {} not found",
                cmd.event_id
            )));
        }

        sqlx::query!(
            "UPDATE kb_cogmaps SET steward_watermark_event_id = $2 WHERE id = $1",
            *cmd.cogmap,
            cmd.event_id,
        )
        .execute(&self.pool)
        .await
        .map_err(api_err)?;

        // A clean watermark advance IS steward-run completion — complete the active job atomically so
        // the concurrency race closes (the watermark only moves on clean completion). A no-op when no
        // job is active (e.g. a manual advance outside the dispatch loop). ApiError → TemperError via `?`.
        crate::services::workflow_job_service::complete(
            &self.pool,
            *cmd.cogmap,
            Persona::Steward.as_str(),
            DispatchType::Steward.as_str(),
        )
        .await?;

        Ok(CommandOutput::new(cmd.event_id))
    }

    async fn steward_dispatch_tick(
        &self,
        cmd: StewardDispatchTick,
    ) -> Result<CommandOutput<Vec<temper_core::types::workflow_job::ClaimedJob>>, TemperError> {
        use crate::services::{steward_service, workflow_job_service};
        use temper_core::types::workflow_job::{
            DEFAULT_STEWARD_DISPATCH_CAP, DEFAULT_STEWARD_LEASE_SECONDS,
        };

        // 1. Reap stale leases (crashed runs → retry/dead) before claiming.
        workflow_job_service::reap(&self.pool, "lease expired").await?;

        // 2. Deterministic sweep over the readable team-joined maps.
        let drifted =
            steward_service::drift_sweep(&self.pool, self.profile_id, cmd.threshold).await?;

        // 3. Enqueue each drifted map (deduped by the in-flight index — already-active maps skip).
        for row in &drifted {
            workflow_job_service::enqueue(
                &self.pool,
                row.cogmap_id,
                Persona::Steward.as_str(),
                DispatchType::Steward.as_str(),
            )
            .await?;
        }

        // 4. Claim up to `cap` for fan-out (one isolated session per claimed job downstream).
        let cap = cmd.cap.unwrap_or(DEFAULT_STEWARD_DISPATCH_CAP) as i32;
        let claimed = workflow_job_service::claim(
            &self.pool,
            Persona::Steward.as_str(),
            DispatchType::Steward.as_str(),
            cap,
            DEFAULT_STEWARD_LEASE_SECONDS,
        )
        .await?;

        Ok(CommandOutput::new(claimed))
    }

    /// Re-materialize a cogmap's regions when its formation delta clears the threshold (T4b). AUTH
    /// BEFORE WRITE: one gated lookup does existence + read-visibility (absent/unreadable → uniform
    /// 404), rides the cogmap-write capability along (readable-but-not-writable → 403), and reads the
    /// current materialize watermark in the same round-trip. The threshold gate is a cheap event count
    /// — deliberately cheaper than the load-and-cluster it guards — so below threshold this is an
    /// idempotent no-op (`materialized: false`) that never touches the substrate. At/above threshold it
    /// delegates to the existing incremental-materialize path (which fires `region_materialized` and
    /// advances `shape_materialized_event_id` via its projection); NO new clustering logic lives here.
    async fn materialize_on_threshold(
        &self,
        cmd: MaterializeOnThreshold,
    ) -> Result<CommandOutput<MaterializeAck>, TemperError> {
        let row = sqlx::query!(
            r#"
            SELECT cogmap_authorable_by_profile($2, $1) AS "can_write!",
                   shape_materialized_event_id AS "watermark: uuid::Uuid"
              FROM kb_cogmaps
             WHERE id = $1
               AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
            "#,
            *cmd.cogmap,
            *self.profile_id,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(api_err)?
        .ok_or_else(|| {
            TemperError::NotFound(format!("cognitive map {} not found", cmd.cogmap.uuid()))
        })?;
        if !row.can_write {
            return Err(TemperError::Forbidden);
        }

        let formation_events = temper_substrate::replay::formation_touched_count_since(
            &self.pool,
            cmd.cogmap.uuid(),
            row.watermark,
        )
        .await
        .map_err(api_err)?;
        let threshold = cmd.threshold.unwrap_or(DEFAULT_MATERIALIZE_THRESHOLD);

        if formation_events < threshold {
            return Ok(CommandOutput::new(MaterializeAck {
                cogmap_id: cmd.cogmap.uuid(),
                materialized: false,
                formation_events,
                threshold,
                regions: None,
                membership_fingerprint: None,
            }));
        }

        // Attribute the materialize to the entity that seeded this cogmap — the earliest map-anchored
        // event's emitter, a real referent (mirrors the substrate harness). At/above threshold there is
        // at least one map-anchored formation event, so this always resolves.
        let emitter: uuid::Uuid = sqlx::query_scalar!(
            r#"
            SELECT emitter_entity_id AS "emitter!"
              FROM kb_events
             WHERE producing_anchor_table = 'kb_cogmaps' AND producing_anchor_id = $1
             ORDER BY occurred_at ASC
             LIMIT 1
            "#,
            *cmd.cogmap,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(api_err)?;

        let outcome = temper_substrate::write::incremental_materialize_cogmap(
            &self.pool,
            cmd.cogmap,
            DEFAULT_MATERIALIZE_LENS,
            EntityId::from(emitter),
        )
        .await
        .map_err(api_err)?;

        Ok(CommandOutput::new(MaterializeAck {
            cogmap_id: cmd.cogmap.uuid(),
            materialized: true,
            formation_events,
            threshold,
            regions: Some(outcome.regions as i64),
            membership_fingerprint: Some(outcome.membership_fingerprint),
        }))
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

        let live = readback::kernel_slice(&self.pool, CogmapId::from(cogmap_uuid))
            .await
            .map_err(api_err)?;

        // The resolvable set: stable ids of live resources ∪ this request's entry ids (bare uuids
        // from the wire), so key on the inner uuid of the typed `resource_id`.
        let mut known: HashSet<uuid::Uuid> = HashSet::new();
        for r in &live {
            known.insert(r.resource_id.uuid());
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

/// Map a `set_facet_with` write error: a unique-violation (SQLSTATE `23505`, the
/// `uq_kb_properties_active` guard) means an active facet with this key is already set on the
/// resource → [`TemperError::Conflict`] (409), not a 500 — the caller must fold the prior facet
/// before re-setting (the steward's fold-then-set loop, D8). Any other error stays a 500
/// ([`api_err`]). The substrate write returns `anyhow::Error`, so the sqlx error is found by
/// walking the source chain rather than a single downcast.
fn map_facet_write_err(e: anyhow::Error) -> TemperError {
    for cause in e.chain() {
        if let Some(sqlx::Error::Database(db)) = cause.downcast_ref::<sqlx::Error>() {
            if db.code().as_deref() == Some("23505") {
                return TemperError::Conflict(
                    "a facet with this key is already set on the resource; fold it before re-setting"
                        .to_string(),
                );
            }
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
