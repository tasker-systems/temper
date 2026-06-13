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
pub mod source;

use std::collections::HashMap;

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::content::{PreparedBlock, PreparedChunk};
use crate::events::{self, SeedAction};
use crate::ids::{BlockId, ChunkId, ResourceId};
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

/// Synthesize the `temper_next` substrate from current `public.*` state.
///
/// WS6 chunk-2: bootstrap (§1/§2) + the resource pass (§8/§2/§1c). Each active resource backfills as
/// one `resource_created` carrying a single up-front content block whose chunks reproduce the
/// production chunk-set verbatim (content, sha256 content_hash, header_path/heading_depth, bge-768
/// embedding). Homes anchor at the resource's remapped context (`('kb_contexts', ctx)`) carrying its
/// originator/owner. The property (§7) and edge (§4) passes land in the following tasks.
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
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await?;

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
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await?;

    for r in selected {
        let new_id = *state.resource_id_by_old.get(&r.id).with_context(|| {
            format!(
                "resource {} absent from resource remap (property pass)",
                r.id
            )
        })?;

        // Managed keys flow through the §7 fate table; only `Property`-fated keys become rows. `Die`
        // (title/slug/id/context), `Edge` (temper-goal → edge pass, Task 8), and `ReconcileToDocType`
        // (temper-type → the doc_type column already a property) are skipped.
        for (key, value) in manifest_entries(&r.managed_meta) {
            if key_fate::key_fate(key) != key_fate::KeyFate::Property {
                continue;
            }
            fire_property(&mut tx, new_id, key, value, maps.migration_entity).await?;
            report.properties += 1;
        }
        // Every `open_meta` key is a property verbatim (§7) — no fate-table consultation.
        for (key, value) in manifest_entries(&r.open_meta) {
            fire_property(&mut tx, new_id, key, value, maps.migration_entity).await?;
            report.properties += 1;
        }
    }

    tx.commit().await?;

    Ok(report)
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
