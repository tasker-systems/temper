//! Synthesis-from-state (WS6 §0): regenerate the `temper_next` substrate from current production
//! (`public.*`) projected state by firing genesis events, NOT by replaying the old (incomplete)
//! ledger. This module is the explicitly-invoked operation behind the `temper-next synthesize`
//! subcommand — never a migrate-time side effect (§D).
//!
//! Synthesis covers **active state only** (§0): soft-deleted resources are not synthesized. The
//! per-resource sequence (filled in across the WS6 chunk-2 tasks) is: `resource_created` (with
//! block/chunk manifests per §8) → `property_asserted` per surviving manifest key (§7) →
//! `relationship_asserted` per edge (§4); folded rows synthesize as assert+fold event pairs.
//!
//! This file currently carries the scaffolding: [`run`] is a stub returning an empty [`SynthReport`];
//! the typed `public.*` reads live in [`source`].

pub mod bootstrap;
pub mod key_fate;
pub mod parity;
pub mod source;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::affinity::EdgeKind;
use crate::content::{PreparedBlock, PreparedChunk};
use crate::events::{self, EdgeHome, SeedAction};
use crate::ids::{BlockId, ChunkId, ContextId, ResourceId};
use crate::payloads;

/// Knobs for a synthesis run.
#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    /// Stop after N resources (rehearsal); `0` = all.
    pub limit: usize,
}

/// Counts produced by a synthesis run. Later tasks extend this as each pass lands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SynthReport {
    /// Resources synthesized (`resource_created` fired).
    pub resources: usize,
    /// Properties synthesized (`property_asserted` fired).
    pub properties: usize,
    /// Edges synthesized (`relationship_asserted` fired).
    pub edges: usize,
}

/// The old→new remaps accumulated across synthesis passes. [`run`] threads this so the property pass
/// (§7) and the edge pass (§4) can resolve a production resource id to its synthesized id.
#[derive(Debug, Clone, Default)]
pub struct SynthState {
    /// production `kb_resources.id` → synthesized `temper_next.kb_resources.id` (the resource pass).
    pub resource_id_by_old: HashMap<Uuid, ResourceId>,
}

/// Begin a synthesis write transaction with the two shared session settings every pass needs:
///
/// 1. `search_path = temper_next, public` — so the SQL mutation functions + triggers resolve their
///    unqualified references into `temper_next` (same discipline as `bootstrap::run`).
/// 2. `idle_in_transaction_session_timeout = 0` — each pass is one atomic transaction over the full
///    corpus driven by sequential per-item round-trips (one `resource_create` / `facet_set` /
///    `relationship_assert` per statement). Against a managed Postgres that reaps idle-in-transaction
///    sessions (Neon caps the timeout at 5min), the transaction sits "idle in transaction" between
///    statements and gets killed mid-pass. That timeout guards against *leaked interactive*
///    transactions, not a deliberate, explicitly-invoked bulk migration — so disable it here.
///    Discovered by the WS6 flip Neon-branch rehearsal (the §D final synthesis would have failed
///    identically over the production corpus).
async fn begin_synthesis_tx(pool: &PgPool) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL idle_in_transaction_session_timeout = 0")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// Synthesize the `temper_next` substrate from current `public.*` state.
///
/// WS6 chunk-2: bootstrap (§1/§2) + the resource pass (§8/§2/§1c). Each active resource backfills as
/// one `resource_created` carrying a single up-front content block whose chunks reproduce the
/// production chunk-set verbatim (content, sha256 content_hash, header_path/heading_depth, bge-768
/// embedding). Homes anchor at the resource's remapped context (`('kb_contexts', ctx)`) carrying its
/// originator/owner. The property pass (§7) then writes each surviving manifest key as a
/// `kb_properties` row, and the edge pass (§4) synthesizes one `relationship_asserted` per live
/// `kb_resource_edges` row (kind/polarity/label/weight verbatim, folded rows as an assert+fold pair)
/// plus the minted `temper-goal` → goal→task edges (G8), deduped against the materialized edges.
pub async fn run(pool: &PgPool, opts: RunOpts) -> Result<SynthReport> {
    let resources = source::active_resources(pool).await?;
    let maps = bootstrap::run(pool, &resources).await?;

    let take = if opts.limit == 0 {
        resources.len()
    } else {
        opts.limit.min(resources.len())
    };
    let selected = &resources[..take];

    // Pre-fetch each resource's chunks off the bare pool (public.* reads are schema-qualified) BEFORE
    // opening the temper_next write transaction, so reads and writes never share a connection's state.
    let mut chunk_sets: Vec<Vec<source::SourceChunk>> = Vec::with_capacity(selected.len());
    for r in selected {
        chunk_sets.push(source::chunks_for(pool, r.id).await?);
    }

    let mut state = SynthState::default();
    let mut report = SynthReport::default();

    // All temper_next writes run in one transaction with `search_path = temper_next, public` so the
    // SQL functions + triggers resolve their unqualified references into temper_next (same discipline
    // as bootstrap::run). Each `resource_create` fires through the single `events::fire` surface.
    let mut tx = begin_synthesis_tx(pool).await?;

    for (r, chunks) in selected.iter().zip(&chunk_sets) {
        let block = single_block_from_chunks(chunks);

        let ctx = *maps
            .context_id_by_old
            .get(&r.kb_context_id)
            .with_context(|| {
                format!(
                    "resource {} home context {} absent from bootstrap context remap",
                    r.id, r.kb_context_id
                )
            })?;
        let owner = *maps
            .profile_id_by_old
            .get(&r.owner_profile_id)
            .with_context(|| {
                format!(
                    "resource {} owner profile {} absent from bootstrap profile remap",
                    r.id, r.owner_profile_id
                )
            })?;
        let originator = *maps
            .profile_id_by_old
            .get(&r.originator_profile_id)
            .with_context(|| {
                format!(
                    "resource {} originator profile {} absent from bootstrap profile remap",
                    r.id, r.originator_profile_id
                )
            })?;

        let fired = events::fire(
            &mut tx,
            SeedAction::ResourceCreate {
                title: &r.title,
                origin_uri: &r.origin_uri,
                // Preserve the production resource id verbatim (PR#124 identity-as-input) so the
                // decorated `ref` (which embeds this id) survives the hard cutover — re-minting
                // here would dangle every externally-held ref at the flip.
                resource_id: Some(ResourceId::from(r.id)),
                home: payloads::AnchorRef::context(ctx),
                owner,
                originator: Some(originator),
                blocks: std::slice::from_ref(&block),
                doc_type: Some(r.doc_type.as_str()),
                emitter: maps.migration_entity,
            },
        )
        .await?;
        let new_id = fired.resource()?;
        state.resource_id_by_old.insert(r.id, new_id);
        report.resources += 1;
    }

    tx.commit().await?;

    // `state.resource_id_by_old` is the remap the property (§7) and edge (§4) passes consume; built
    // above so they thread the old→new resource ids without re-reading the synthesized rows.
    debug_assert_eq!(state.resource_id_by_old.len(), report.resources);

    // ── property pass (§7) ───────────────────────────────────────────────────────────────────────
    // Each surviving manifest key becomes a `kb_properties` row per the §7 fate table. This runs AFTER
    // the resource pass commits: `facet_set` anchors each `property_asserted` event on the owner
    // resource's home (erroring if homeless), and the homes are now durable. A fresh transaction (homes
    // already committed) mirrors the resource-pass search_path discipline so the SQL functions resolve
    // their unqualified references into `temper_next`.
    let mut tx = begin_synthesis_tx(pool).await?;

    for r in selected {
        let new_id = *state.resource_id_by_old.get(&r.id).with_context(|| {
            format!(
                "resource {} absent from resource remap (property pass)",
                r.id
            )
        })?;

        // §7 property sources, in order: managed keys flow through the fate table (only `Property`-fated
        // keys become rows — `Die` title/slug/id/context, `Edge` temper-goal, and `ReconcileToDocType`
        // temper-type are skipped); then every `open_meta` key verbatim (no fate consultation).
        let managed = manifest_entries(&r.managed_meta)
            .filter(|(key, _)| key_fate::key_fate(key) == key_fate::KeyFate::Property);
        // Dedup identical `(key, value)` pairs across the two sources: `kb_properties`' active grain is
        // `(owner, property_key, property_value)`, so the SAME assertion appearing in both manifests
        // (observed in production: a `date` key carried in both `managed_meta` and `open_meta` with an
        // equal value) is ONE property, not a `uq_kb_properties_active` violation. Distinct values for a
        // repeated key are different pairs and still both fire — multi-valued keys are preserved. Linear
        // scan over a per-resource list (a handful of keys); `serde_json::Value` is not `Hash`, but its
        // `PartialEq` is the same semantic JSONB equality the constraint enforces.
        let mut fired: Vec<(&str, &serde_json::Value)> = Vec::new();
        for (key, value) in managed.chain(manifest_entries(&r.open_meta)) {
            if fired.iter().any(|(k, v)| *k == key && *v == value) {
                continue;
            }
            fired.push((key, value));
            fire_property(&mut tx, new_id, key, value, maps.migration_entity).await?;
            report.properties += 1;
        }
    }

    tx.commit().await?;

    // ── edge pass (§4) + minted temper-goal edges (§7/G8) ─────────────────────────────────────────
    // Per live `public.kb_resource_edges` row: synthesize a `relationship_asserted` carrying kind,
    // polarity, label, and weight verbatim, with both endpoints remapped to their synthesized ids and
    // the edge homed at its SOURCE endpoint's remapped context (§1c — `public.kb_resource_edges` has no
    // home column, and a resource↔resource edge homes in the shared context; for a cross-context edge
    // the source's context is the deterministic choice). A folded row synthesizes as the assert+fold
    // pair. Then each task carrying a non-empty `temper-goal` mints the goal→task edge the production
    // frontmatter-edge projection emits (`Contains`/`forward`/`parent_of`/`1.0`, G8), DEDUPED against
    // any edge already synthesized from `kb_resource_edges` (keyed on `(src, tgt, kind, label)`).
    let source_edges = source::edges(pool).await?;

    // Each selected resource's remapped home context — the edge home lookup (by source endpoint).
    let mut ctx_by_resource: HashMap<Uuid, ContextId> = HashMap::new();
    for r in selected {
        if let Some(ctx) = maps.context_id_by_old.get(&r.kb_context_id) {
            ctx_by_resource.insert(r.id, *ctx);
        }
    }

    // Dedup set of synthesized edges, keyed on the new endpoints + kind + label so a minted
    // temper-goal edge already present as a materialized `kb_resource_edges` row is not double-created.
    let mut seen: HashSet<(Uuid, Uuid, EdgeKind, Option<String>)> = HashSet::new();

    let mut tx = begin_synthesis_tx(pool).await?;

    for e in &source_edges {
        // `source::edges` already restricts to active↔active; an endpoint can still be absent here only
        // if a `RunOpts::limit` excluded the resource from the synthesized set — skip such edges.
        let (Some(&src), Some(&tgt)) = (
            state.resource_id_by_old.get(&e.source),
            state.resource_id_by_old.get(&e.target),
        ) else {
            continue;
        };
        let kind = EdgeKind::from_sql(&e.edge_kind).with_context(|| {
            format!("edge {} has unrecognized edge_kind {:?}", e.id, e.edge_kind)
        })?;
        let polarity = payloads::EdgePolarity::from_sql(&e.polarity)
            .with_context(|| format!("edge {} has unrecognized polarity {:?}", e.id, e.polarity))?;
        // An empty production label carries as no label (the payload's `Option<String>` is
        // skip-if-none, so the projection writes NULL — never an empty string).
        let label = (!e.label.is_empty()).then(|| e.label.clone());
        let home = *ctx_by_resource.get(&e.source).with_context(|| {
            format!(
                "edge {} source {} absent from context home map",
                e.id, e.source
            )
        })?;

        let fired = events::fire(
            &mut tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind,
                polarity,
                label: label.as_deref(),
                weight: e.weight,
                home: EdgeHome::Context(home),
                emitter: maps.migration_entity,
            },
        )
        .await?;
        seen.insert((src.uuid(), tgt.uuid(), kind, label.clone()));
        report.edges += 1;

        // The one folded edge synthesizes as an assert + fold pair (§4): fold the edge just asserted.
        if e.is_folded {
            let edge_id = fired.relationship()?;
            events::fire(
                &mut tx,
                SeedAction::RelationshipFold {
                    edge: edge_id,
                    reason: None,
                    emitter: maps.migration_entity,
                },
            )
            .await?;
        }
    }

    // Minted temper-goal edges (§7/G8). The goal reference (slug or trailing-uuid) resolves against the
    // ACTIVE resource set; `goal→task` is the §7 reversal (source = goal). Skipped when the dedup set
    // already carries it (the materialized-edge case — production's 68 `contains` rows vs 363 keys).
    let slug_to_old: HashMap<&str, Uuid> = resources
        .iter()
        .filter_map(|r| r.slug.as_deref().map(|s| (s, r.id)))
        .collect();
    let active_ids: HashSet<Uuid> = resources.iter().map(|r| r.id).collect();

    for r in selected {
        let Some(goal_ref) = r
            .managed_meta
            .get("temper-goal")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(goal_old) = resolve_goal_target(goal_ref, &slug_to_old, &active_ids) else {
            continue; // unresolvable / external / forward reference — not an edge (CONFORM TargetRef::parse)
        };
        // goal → task (the §7 reversal): source = the goal, target = this task.
        let (Some(&src), Some(&tgt)) = (
            state.resource_id_by_old.get(&goal_old),
            state.resource_id_by_old.get(&r.id),
        ) else {
            continue;
        };
        let kind = EdgeKind::Contains;
        let label = Some("parent_of".to_owned());
        if seen.contains(&(src.uuid(), tgt.uuid(), kind, label.clone())) {
            continue; // already synthesized from kb_resource_edges — do not double-create (dedup)
        }
        let home = *ctx_by_resource.get(&goal_old).with_context(|| {
            format!("minted temper-goal edge source {goal_old} absent from context home map")
        })?;
        events::fire(
            &mut tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind,
                polarity: payloads::EdgePolarity::Forward,
                label: label.as_deref(),
                weight: 1.0,
                home: EdgeHome::Context(home),
                emitter: maps.migration_entity,
            },
        )
        .await?;
        seen.insert((src.uuid(), tgt.uuid(), kind, label));
        report.edges += 1;
    }

    tx.commit().await?;

    // ── §8 body-text parity gate ─────────────────────────────────────────────────────────────────
    // Before reporting success, prove that reconstructing each synthesized resource's body (the
    // production `get_content` algorithm over `temper_next` chunks) reproduces the body production
    // serves today, per resource (§8: "before cutover proceeds"). This is a TEXT comparison — never the
    // two `body_hash` columns, which differ by construction (structural merkle vs markdown sha256). A
    // mismatch is fatal: `run` refuses to report success so an upstream cutover cannot proceed on a
    // silently-diverged substrate.
    let parity = parity::body_parity_report(pool).await?;
    if !parity.is_clean() {
        anyhow::bail!(
            "body-text parity gate failed (§8): {} of {} synthesized resources diverge from production: {:?}",
            parity.mismatches.len(),
            parity.checked,
            parity.mismatched_ids(),
        );
    }

    Ok(report)
}

/// Resolve a `temper-goal` reference to a production resource id, CONFORMing to edge_service's
/// `TargetRef::parse`: a parseable UUID resolves by id (kept only if it names an active resource), an
/// `http(s)` URL is not a graph edge, otherwise it is a slug resolved against the active resource set.
/// `None` ⇒ no edge is minted (unresolvable / external / a forward reference whose target isn't live).
fn resolve_goal_target(
    value: &str,
    slug_to_old: &HashMap<&str, Uuid>,
    active_ids: &HashSet<Uuid>,
) -> Option<Uuid> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(uuid) = Uuid::parse_str(trimmed) {
        return active_ids.contains(&uuid).then_some(uuid);
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return None;
    }
    slug_to_old.get(trimmed).copied()
}

/// Iterate a manifest field's object entries (`managed_meta`/`open_meta`). A non-object (null/absent)
/// manifest field yields no entries — defensive against a resource with no manifest meta.
fn manifest_entries(meta: &serde_json::Value) -> impl Iterator<Item = (&str, &serde_json::Value)> {
    meta.as_object()
        .into_iter()
        .flat_map(|m| m.iter().map(|(k, v)| (k.as_str(), v)))
}

/// Fire one `property_asserted` for `key`/`value` on a synthesized resource, through the single
/// `events::fire` surface (reusing the key-agnostic `facet_set` SQL). `weight` is `1.0` — synthesis
/// carries each manifest key at full assertion weight.
async fn fire_property(
    tx: &mut sqlx::PgConnection,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: crate::ids::EntityId,
) -> Result<()> {
    events::fire(
        tx,
        SeedAction::PropertyAssert {
            resource,
            key,
            value,
            weight: 1.0,
            emitter,
        },
    )
    .await?;
    Ok(())
}

/// Build the §8 single up-front content block (seq 0, no role) from a resource's production chunk-set,
/// carrying every chunk VERBATIM: pre-generated identities, the production `content_hash`, prose,
/// bge-768 `embedding`, and the `header_path`/`heading_depth` render metadata. Nothing is re-chunked or
/// re-embedded — embeddings are non-replayed derived state, carried as-is (§8).
fn single_block_from_chunks(chunks: &[source::SourceChunk]) -> PreparedBlock {
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: chunks
            .iter()
            .map(|c| PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: c.chunk_index,
                content_hash: c.content_hash.clone(),
                content: c.content.clone(),
                embedding: c.embedding.clone(),
                // Production columns are NOT NULL (`header_path` defaults `''`, `heading_depth` `0`);
                // carry them verbatim so a downstream read reconstructs headed markdown identically.
                header_path: Some(c.header_path.clone()),
                heading_depth: Some(c.heading_depth),
            })
            .collect(),
    }
}
