//! Replay primitives (payload spec §7.2): walk the ledger through the SAME `_project_*` halves
//! normal operation uses, into a freshly reset namespace, and prove the projections come back
//! byte-identical. The masked-surrogate rule: tables whose `id` carries no inbound references —
//! `kb_resource_homes`, `kb_properties`, `kb_block_revisions` — compare with `id` masked, ordered by
//! natural key; every *referenced* identity is payload-carried (identity-as-input), so all other
//! tables diff in full, ids and timestamps included (projected timestamps come from the event's
//! `occurred_at`, never `now()`).
//!
//! Region tables are excluded from the diff: they are second-order derived compute (clustering
//! output). Their proof is re-materialization — the membership fingerprint must equal the one
//! recorded in the `region_materialized` payload.
//!
//! Dumps/restores are dynamic-table operations, so this module uses runtime `sqlx::query` (the
//! established exception class) rather than compile-checked macros.

use crate::events::EventKind;
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use temper_core::types::home::HomeAnchor;
use uuid::Uuid;

/// (table, dump-query) — canonical, deterministic row dumps. Masked tables subtract 'id' and order
/// by natural key; everything else orders by id.
const PROJECTION_DUMPS: &[(&str, &str)] = &[
    (
        "kb_resources",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_resources t",
    ),
    (
        "kb_resource_homes",
        "SELECT coalesce(jsonb_agg((to_jsonb(t) - 'id') ORDER BY t.resource_id), '[]'::jsonb) FROM kb_resource_homes t",
    ),
    (
        "kb_cogmaps",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_cogmaps t",
    ),
    (
        "kb_content_blocks",
        // mask current_revision_id: it points at a kb_block_revisions.id, which is freshly minted per
        // projection (that table masks its own id for the same reason), so it differs fire-vs-replay.
        "SELECT coalesce(jsonb_agg((to_jsonb(t) - 'current_revision_id') ORDER BY t.id), '[]'::jsonb) FROM kb_content_blocks t",
    ),
    (
        "kb_chunks",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_chunks t",
    ),
    (
        "kb_chunk_content",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.chunk_id), '[]'::jsonb) FROM kb_chunk_content t",
    ),
    (
        "kb_block_revisions",
        // include `created` (the event occurred_at — replay-stable) in the mask order: a block revised
        // back to a prior body produces two revisions with the SAME (block_id, block_body_hash), and
        // ordering on that pair alone is a tie whose jsonb_agg order can differ fire-vs-replay.
        "SELECT coalesce(jsonb_agg((to_jsonb(t) - 'id') ORDER BY t.block_id, t.block_body_hash, t.created), '[]'::jsonb) FROM kb_block_revisions t",
    ),
    (
        "kb_block_content",
        // mask block_revision_id (a masked kb_block_revisions.id) and order by the bytes themselves, so
        // the comparison is over the SET of (content, content_hash) stored — the meaningful equivalence
        // (the same bytes survive), independent of which non-deterministic revision id they hang off.
        "SELECT coalesce(jsonb_agg((to_jsonb(t) - 'block_revision_id') ORDER BY t.content_hash, t.content), '[]'::jsonb) FROM kb_block_content t",
    ),
    (
        "kb_properties",
        "SELECT coalesce(jsonb_agg((to_jsonb(t) - 'id') ORDER BY t.owner_table, t.owner_id, t.property_key, t.property_value), '[]'::jsonb) FROM kb_properties t",
    ),
    (
        "kb_edges",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_edges t",
    ),
    (
        "kb_cogmap_lenses",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_cogmap_lenses t",
    ),
    (
        "kb_invocations",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_invocations t",
    ),
];

/// Non-projected input tables, copied verbatim into the replay namespace. Restore order respects FK
/// dependencies. `kb_team_cogmaps` restores AFTER the event walk (it references projected cogmaps).
/// Teams (+ DAG) restore BEFORE profiles: the personal-team trigger fires per restored profile and
/// must no-op by slug against the restored originals, keeping original team ids intact for the
/// kb_team_members restore (WS6 §2).
const INPUT_TABLES: &[&str] = &[
    "kb_teams",
    "kb_teams_parents",
    "kb_profiles",
    "kb_entities",
    "kb_team_members",
    "kb_contexts",
    "kb_team_contexts",
    // Grants are INPUT, not projection: replay restores kb_access_grants verbatim (the 5 pre-epoch
    // grants have no events, so rebuilding from events would report them spurious forever). The
    // grant_created/grant_revoked walk arms are no-ops (see the match below) precisely because the
    // final grant state rides in here — spec 2026-07-16 §7, Task 5 Step 5.
    "kb_access_grants",
    "kb_topics",
    "kb_event_types",
    "kb_events",
];

#[derive(Debug)]
pub struct LedgerSnapshot {
    inputs: Vec<(String, serde_json::Value)>,
    team_cogmaps: serde_json::Value,
    /// event id → content sidecar for the content-bearing types, reconstructed from the CAS
    /// (kb_chunk_content prose + the stored chunk embedding as pgvector text — a derived-cache
    /// carry-over so the diff stays total without re-running ONNX).
    sidecars: HashMap<Uuid, serde_json::Value>,
}

/// Canonical dumps of every replay-diffed projection table.
pub async fn dump_projections(pool: &PgPool) -> Result<Vec<(String, serde_json::Value)>> {
    let mut out = Vec::new();
    for (table, q) in PROJECTION_DUMPS {
        let v: serde_json::Value = sqlx::query_scalar(q).fetch_one(pool).await?;
        out.push((table.to_string(), v));
    }
    Ok(out)
}

async fn dump_table(pool: &PgPool, table: &str) -> Result<serde_json::Value> {
    let q =
        format!("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t), '[]'::jsonb) FROM {table} t");
    Ok(sqlx::query_scalar(&q).fetch_one(pool).await?)
}

/// Capture everything replay needs, BEFORE the namespace reset. Also asserts the CAS retention
/// invariant (proof obligation 4): every chunk a content-bearing payload references must still have
/// its kb_chunk_content row — fold/supersede affect visibility, never existence.
pub async fn snapshot(pool: &PgPool) -> Result<LedgerSnapshot> {
    let mut inputs = Vec::new();
    for t in INPUT_TABLES {
        inputs.push((t.to_string(), dump_table(pool, t).await?));
    }
    let team_cogmaps = dump_table(pool, "kb_team_cogmaps").await?;

    // sidecars for the content-bearing events: payload manifests → chunk ids → CAS lookups
    let mut sidecars = HashMap::new();
    let rows = sqlx::query(
        "SELECT e.id, et.name, e.payload \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name IN ('cogmap_seeded','resource_created','block_created','block_mutated','charter_set') ORDER BY e.id",
    )
    .fetch_all(pool)
    .await?;
    for r in rows {
        let event_id: Uuid = r.get(0);
        let name: String = r.get(1);
        let payload: serde_json::Value = r.get(2);
        let kind = EventKind::from_canonical_name(&name)
            .with_context(|| format!("snapshot: unknown event type {name}"))?;
        let manifests = match kind {
            EventKind::CogmapSeeded => payload.pointer("/telos/blocks").cloned(),
            // charter_set carries the role-tagged block set at the top level (CharterSet::blocks),
            // same shape as resource_created — the chunk content/embeddings must be retained for replay.
            EventKind::ResourceCreated | EventKind::CharterSet => payload.get("blocks").cloned(),
            // block_created carries one block manifest under `block`; wrap it as a
            // single-element block array so the chunk-extraction loop stays uniform.
            EventKind::BlockCreated => payload
                .get("block")
                .cloned()
                .map(|block| serde_json::json!([block])),
            // block_mutated carries a flat `chunks` array (one block); wrap it as a single
            // pseudo-block so the chunk-extraction loop below stays uniform.
            EventKind::BlockMutated => payload
                .get("chunks")
                .cloned()
                .map(|chunks| serde_json::json!([{ "chunks": chunks }])),
            // The SQL query above restricts to the three content-bearing types, so the
            // remaining variants are unreachable here — they carry no chunk manifests.
            EventKind::ResourceUpdated
            | EventKind::SalienceRefreshed
            | EventKind::ResourceDeleted
            | EventKind::ResourceRehomed
            // A segmented ingest's completion assertion. It carries no content — the bytes arrived on
            // the block_created events it is attesting to — so it needs no sidecar.
            | EventKind::ResourceFinalized
            | EventKind::RelationshipAsserted
            | EventKind::RelationshipRetyped
            | EventKind::RelationshipReweighted
            | EventKind::PropertyAsserted
            | EventKind::PropertySet
            | EventKind::LensCreated
            | EventKind::RegionMaterialized
            | EventKind::RelationshipFolded
            | EventKind::BlockProvenanceAnnotated
            | EventKind::ResourceReassigned
            | EventKind::ContextReassigned
            | EventKind::DelegatedLaunch
            | EventKind::InvocationClosed
            // Admin-ledger events (NULL-anchored, spec 2026-07-16): no content, no sidecar.
            | EventKind::AdminLedgerOpened
            | EventKind::GrantCreated
            | EventKind::GrantRevoked
            | EventKind::SlackPrincipalDisconnected => None,
        }
        .context("content-bearing payload missing blocks")?;
        let mut side = serde_json::Map::new();
        for block in manifests.as_array().context("blocks not an array")? {
            for chunk in block["chunks"].as_array().context("chunks not an array")? {
                let chunk_id: Uuid = chunk["chunk_id"]
                    .as_str()
                    .context("chunk_id missing")?
                    .parse()?;
                let row = sqlx::query(
                    "SELECT cc.content, c.embedding::text, c.embedded_with \
                       FROM kb_chunk_content cc JOIN kb_chunks c ON c.id = cc.chunk_id \
                      WHERE cc.chunk_id = $1",
                )
                .bind(chunk_id)
                .fetch_one(pool)
                .await
                .with_context(|| {
                    format!("CAS retention violated: chunk {chunk_id} has no content row")
                })?;
                let content: String = row.get(0);
                let embedding: Option<String> = row.get(1);
                // `embedded_with` is sidecar-only (it is NOT on the ledger payload), so replay can
                // only recover it from the projected row — exactly as it already does for the vector
                // itself. Omitting it would make every replayed chunk land NULL: the projection stops
                // being byte-identical (which is what the replay-roundtrip tests assert), and a real
                // rebuild-from-ledger would silently discard all provenance and re-stale the whole
                // index.
                let embedded_with: Option<String> = row.get(2);
                side.insert(
                    chunk_id.to_string(),
                    serde_json::json!({
                        "content": content,
                        "embedding": embedding,
                        "embedded_with": embedded_with,
                    }),
                );
            }
        }
        // Re-supply the `__blocks` sidecar (verbatim block bytes, PR 3) from kb_block_content, keyed
        // EXACTLY as the fire path keyed it: by block id for a mutate, by seq for create/append/charter.
        // The revision a given event created is found by (block_id, this event's occurred_at) — the
        // revision id itself is non-deterministic and never appears in the payload. A block that stored
        // no bytes (a `derived` block: charter/scenario) simply yields no row and is omitted, so replay
        // reproduces the same coverage — and therefore the same body_storage.
        let block_keys: Vec<(String, Uuid)> = match kind {
            EventKind::BlockMutated => {
                let bid: Uuid = payload["block_id"]
                    .as_str()
                    .context("block_mutated payload missing block_id")?
                    .parse()?;
                vec![(bid.to_string(), bid)]
            }
            // create/append/charter: key by seq, look up by block id — both live on the manifest.
            _ => manifests
                .as_array()
                .context("blocks not an array")?
                .iter()
                .filter_map(|b| {
                    let bid: Uuid = b["block_id"].as_str()?.parse().ok()?;
                    let seq = b["seq"].as_i64()?;
                    Some((seq.to_string(), bid))
                })
                .collect(),
        };
        let mut blocks_side = serde_json::Map::new();
        for (key, block_id) in block_keys {
            // Map this event to the revision IT created, by (block_id, this event's occurred_at). This
            // assumes ONE content-bearing revision of a block per event's occurred_at. `now()` is
            // transaction-scoped, so two revisions of one block minted in a SINGLE transaction would
            // share `created` and this lookup would become ambiguous — no current write path does that
            // (one content event per transaction), but rather than silently pick an arbitrary revision
            // (which would corrupt the replay proof, not fail it), assert the 1:1 mapping and fail loud
            // if a future batched-mutation path ever violates it.
            let rows = sqlx::query(
                "SELECT bc.content, bc.content_hash \
                   FROM kb_block_revisions r JOIN kb_block_content bc ON bc.block_revision_id = r.id \
                  WHERE r.block_id = $1 \
                    AND r.created = (SELECT occurred_at FROM kb_events WHERE id = $2)",
            )
            .bind(block_id)
            .bind(event_id)
            .fetch_all(pool)
            .await?;
            match rows.as_slice() {
                // No stored bytes for this event's revision (a `derived` block) — nothing to re-supply.
                [] => {}
                [row] => {
                    let content: String = row.get(0);
                    let content_hash: String = row.get(1);
                    blocks_side.insert(
                        key,
                        serde_json::json!({ "content": content, "content_hash": content_hash }),
                    );
                }
                _ => anyhow::bail!(
                    "replay: ambiguous block-content reconstruction — block {block_id} has {} content \
                     revisions sharing event {event_id}'s occurred_at, so (block_id, occurred_at) no \
                     longer maps to a single revision (a batched multi-revision transaction?). Failing \
                     loud rather than re-supplying arbitrary bytes.",
                    rows.len()
                ),
            }
        }
        if !blocks_side.is_empty() {
            side.insert(
                "__blocks".to_string(),
                serde_json::Value::Object(blocks_side),
            );
        }
        sidecars.insert(event_id, serde_json::Value::Object(side));
    }
    Ok(LedgerSnapshot {
        inputs,
        team_cogmaps,
        sidecars,
    })
}

async fn restore_table(pool: &PgPool, table: &str, rows: &serde_json::Value) -> Result<()> {
    // kb_profiles INSERTs fire the sync_system_membership trigger, which may insert team_members
    // rows that the verbatim restore then re-inserts — tolerate the collision there.
    let conflict = if table == "kb_team_members" {
        " ON CONFLICT (team_id, profile_id) DO NOTHING"
    } else {
        ""
    };
    let q = format!(
        "INSERT INTO {table} SELECT * FROM jsonb_populate_recordset(NULL::{table}, $1){conflict}"
    );
    sqlx::query(&q).bind(rows).execute(pool).await?;
    Ok(())
}

/// Restore inputs and walk the ledger through the projection halves — the SAME code normal
/// operation runs. Call against a freshly reset (01+02, un-seeded) namespace.
pub async fn replay(pool: &PgPool, snap: &LedgerSnapshot) -> Result<()> {
    for (table, rows) in &snap.inputs {
        restore_table(pool, table, rows).await?;
    }
    let events = sqlx::query(
        "SELECT e.id, et.name, e.payload \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id ORDER BY e.id",
    )
    .fetch_all(pool)
    .await?;
    for r in events {
        let id: Uuid = r.get(0);
        let name: String = r.get(1);
        let payload: serde_json::Value = r.get(2);
        let kind = EventKind::from_canonical_name(&name)
            .with_context(|| format!("replay: no projector for event type {name}"))?;
        match kind {
            EventKind::CogmapSeeded => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_cogmap_seeded($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            EventKind::ResourceCreated => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_resource_created($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            EventKind::BlockMutated => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_block_mutated($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            EventKind::BlockCreated => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_block_created($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            EventKind::CharterSet => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_charter_set($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            // A segmented ingest completing. NO sidecar — it carries no content, only the assertion
            // that the body is all here (`resource_finalize` validated that before recording it). Two
            // args, unlike its content-bearing neighbours above.
            EventKind::ResourceFinalized => {
                sqlx::query("SELECT _project_resource_finalized($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::RelationshipAsserted => {
                sqlx::query("SELECT _project_relationship_asserted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::PropertyAsserted => {
                sqlx::query("SELECT _project_property_asserted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::PropertySet => {
                sqlx::query("SELECT _project_property_set($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::LensCreated => {
                sqlx::query("SELECT _project_lens_created($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::RegionMaterialized => {
                sqlx::query("SELECT _project_region_materialized($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            // T6's cheap clock. Projects ONLY the telos snapshot onto the anchor — the readouts it
            // recomputed are derived compute (like the region rows), re-provable by re-derivation.
            EventKind::SalienceRefreshed => {
                sqlx::query("SELECT _project_salience_refreshed($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            // WS6 4c mutations + the relationship_folded sibling (payload-only projectors, no sidecar).
            EventKind::RelationshipFolded => {
                sqlx::query("SELECT _project_relationship_folded($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::RelationshipRetyped => {
                sqlx::query("SELECT _project_relationship_retyped($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::RelationshipReweighted => {
                sqlx::query("SELECT _project_relationship_reweighted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            // Annotate-only provenance (issue #355): payload-only projector, no sidecar — records
            // kb_block_provenance rows, touches no chunks (so replay reprojects it without prose).
            EventKind::BlockProvenanceAnnotated => {
                sqlx::query("SELECT _project_block_annotated($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::ResourceDeleted => {
                sqlx::query("SELECT _project_resource_deleted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::ResourceUpdated => {
                sqlx::query("SELECT _project_resource_updated($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::ResourceRehomed => {
                sqlx::query("SELECT _project_resource_rehomed($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::ResourceReassigned => {
                sqlx::query("SELECT _project_resource_reassigned($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::ContextReassigned => {
                sqlx::query("SELECT _project_context_reassigned($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::DelegatedLaunch => {
                sqlx::query("SELECT _project_delegated_launch($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            EventKind::InvocationClosed => {
                sqlx::query("SELECT _project_invocation_closed($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            // Admin-ledger events are NULL-anchored (the cognition firewall, spec 2026-07-16): they
            // ride kb_events but touch no _project_* cognition half, so the walk is a no-op. The
            // event rows themselves survive replay via the kb_events input table; grant STATE is
            // replayed from the kb_access_grants input table (Task 5), not rebuilt from these events.
            EventKind::AdminLedgerOpened
            | EventKind::GrantCreated
            | EventKind::GrantRevoked
            | EventKind::SlackPrincipalDisconnected => {}
        }
    }
    restore_table(pool, "kb_team_cogmaps", &snap.team_cogmaps).await?;
    Ok(())
}

/// The recorded materialization acts (last per lens):
/// (anchor, lens_id, watermark_event_id, membership_fingerprint) — for the region re-proof.
///
/// The anchor is read from the pair, falling back to the pre-T3 `cogmap_id` key (`kb_events` is
/// append-only, so acts written before the anchor pair existed are immortal — see
/// `last_materialize_event`, which shares the dual-read).
pub async fn recorded_materializations(
    pool: &PgPool,
) -> Result<Vec<(HomeAnchor, Uuid, Uuid, String)>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (e.payload->>'lens_id') \
                coalesce(e.payload->>'home_anchor_table', 'kb_cogmaps'), \
                coalesce((e.payload->>'home_anchor_id')::uuid, (e.payload->>'cogmap_id')::uuid), \
                (e.payload->>'lens_id')::uuid, \
                (e.payload->>'watermark_event_id')::uuid, \
                e.payload->>'membership_fingerprint' \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'region_materialized' \
          ORDER BY e.payload->>'lens_id', e.id DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let table: String = r.get(0);
            let id: Uuid = r.get(1);
            let anchor = HomeAnchor::from_parts(&table, id)
                .with_context(|| format!("region_materialized: unknown anchor table {table:?}"))?;
            Ok((anchor, r.get(2), r.get(3), r.get(4)))
        })
        .collect()
}

/// The CONTENT events: a member's prose moved (new chunk embeddings) without changing any
/// membership input — the readout-only formation inputs (drift §1). `block_created`/`block_folded`
/// are listed forward-compatibly (no mutation fires them yet); when they land they are already a
/// content touch. The readout-refresh gate and the formation gate share this set so they can never
/// disagree on "what is a content touch" (the bug they'd otherwise drift into).
const CONTENT_EVENTS: &[&str] = &["block_mutated", "block_created", "block_folded"];

/// The STRUCTURAL events: they change a region-formation input that lives in the component
/// fingerprint (membership / edges / facets), so they drive re-clustering, not a readout refresh.
const STRUCTURAL_EVENTS: &[&str] = &[
    "resource_created",
    // A delete changes the producer's node set exactly as a create does — `Substrate::load` filters to
    // `is_active`, so a tombstone leaves formation. Listing it here is what makes that filter *land*:
    // without a clock tick the region never re-forms, and the stale membership/centroid/salience simply
    // sits there until some unrelated structural event happens to fire on the anchor. That is precisely
    // why the prod ghost regions were stable rather than self-healing.
    "resource_deleted",
    "cogmap_seeded",
    "relationship_asserted",
    "relationship_retyped",
    "relationship_reweighted",
    "relationship_folded",
    "relationship_decayed",
    "relationship_corrected",
    "property_asserted",
];

/// The most recent `region_materialized` act for `(anchor, lens)`, optionally bounded to events
/// strictly before `before` — the point-in-time a prior projection was computed against.
///
/// **Reads the anchor out of the payload two ways, and that is deliberate.** `kb_events` is
/// append-only: every `region_materialized` event written before T3 carries `cogmap_id` and no
/// anchor pair, and those rows are immortal. Probing only the new key would silently find no prior
/// act on the first pass after deploy — incremental would skip the moved-member readout refresh
/// exactly once, with no error. The `cogmap_id` arm can never false-positive for a context anchor
/// (uuids don't collide across tables), so the dual-read is safe for both regimes.
///
/// One body, two callers (`write::last_materialize_watermark`, `drift::touched_since_last_materialize`)
/// — the probe is load-bearing and two copies would drift.
pub(crate) async fn last_materialize_event<'e, E: sqlx::PgExecutor<'e>>(
    conn: E,
    anchor: HomeAnchor,
    lens_id: Uuid,
    before: Option<Uuid>,
) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'region_materialized' \
           AND (e.payload->>'lens_id')::uuid = $2 \
           AND ( (e.payload->>'home_anchor_id')::uuid = $1 \
              OR (e.payload->>'cogmap_id')::uuid = $1 ) \
           AND ($3::uuid IS NULL OR e.id < $3) \
         ORDER BY e.id DESC LIMIT 1",
    )
    .bind(anchor.uuid())
    .bind(lens_id)
    .bind(before)
    .fetch_optional(conn)
    .await?)
}

/// Did any event whose type is in `names` touch this anchor after `watermark`? The shared body behind
/// the formation and content gates — the anchor-scoping predicate is load-bearing and easy to get
/// wrong, so it lives in exactly one place.
async fn touched_since(
    pool: &PgPool,
    anchor: HomeAnchor,
    watermark: Uuid,
    names: &[&str],
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS ( \
            SELECT 1 FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
             WHERE e.id > $3 \
               AND e.producing_anchor_table = $1 AND e.producing_anchor_id = $2 \
               AND et.name = ANY($4))",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .bind(watermark)
    .bind(names)
    .fetch_one(pool)
    .await?)
}

/// True iff a FORMATION-affecting event (structural ∪ content — the region-formation inputs) touched
/// the cogmap after the given watermark. A recorded fingerprint is only re-provable when this is
/// false — otherwise the recorded act is legitimately stale relative to the substrate (the
/// drift-detection concept), and re-materialization is expected to differ.
pub async fn formation_touched_since(
    pool: &PgPool,
    anchor: HomeAnchor,
    watermark: Uuid,
) -> Result<bool> {
    let names: Vec<&str> = STRUCTURAL_EVENTS
        .iter()
        .chain(CONTENT_EVENTS)
        .copied()
        .collect();
    touched_since(pool, anchor, watermark, &names).await
}

/// COUNT of formation-affecting events (structural ∪ content) that touched the cogmap after
/// `watermark` — the count twin of [`formation_touched_since`]. `watermark == None` counts from the
/// beginning (the cogmap was never materialized), mirroring the `p_watermark IS NULL` gate the
/// steward ingest count uses. Shares the same event-name sets + anchor-scoping predicate as the bool
/// gate (via the shared `touched_since` body's siblings), so the "materialize now?" threshold and the
/// "is the recorded fingerprint stale?" gate can never disagree on what a formation touch is.
///
/// Deliberately CHEAP — one scalar `count(*)` over the anchor-scoped event slice — so it can gate the
/// materialize path without itself being as expensive as the load-and-cluster it guards (T4b).
pub async fn formation_touched_count_since(
    pool: &PgPool,
    anchor: HomeAnchor,
    watermark: Option<Uuid>,
) -> Result<i64> {
    let names: Vec<&str> = STRUCTURAL_EVENTS
        .iter()
        .chain(CONTENT_EVENTS)
        .copied()
        .collect();
    Ok(sqlx::query_scalar(
        "SELECT count(*) \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE ($3::uuid IS NULL OR e.id > $3) \
            AND e.producing_anchor_table = $1 AND e.producing_anchor_id = $2 \
            AND et.name = ANY($4)",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .bind(watermark)
    .bind(&names)
    .fetch_one(pool)
    .await?)
}

/// The RESOURCES whose content moved on this cogmap after `watermark` (distinct) — the members behind
/// each CONTENT event (a block-body revision / add / fold, the readout-only formation inputs), resolved
/// block → owning resource. Incremental materialization refreshes a reused region's readouts only when
/// one of THESE is among its members: a moved member shifts only its own region's centroid, so a
/// content touch that landed in one component must not re-derive another component's readouts (the
/// over-trigger the per-component decomposition removed, kept removed one layer up). Shares the
/// `CONTENT_EVENTS` set + anchor-scoping with [`formation_touched_since`] so the gates can never
/// disagree on "what is a content touch". Empty ⇒ no readout-refresh work this pass.
pub async fn content_touched_resources_since(
    pool: &PgPool,
    anchor: HomeAnchor,
    watermark: Uuid,
) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT b.resource_id \
           FROM kb_events e \
           JOIN kb_event_types et ON et.id = e.event_type_id \
           JOIN kb_content_blocks b ON b.id = (e.payload->>'block_id')::uuid \
          WHERE e.id > $3 \
            AND e.producing_anchor_table = $1 AND e.producing_anchor_id = $2 \
            AND et.name = ANY($4)",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .bind(watermark)
    .bind(CONTENT_EVENTS)
    .fetch_all(pool)
    .await?)
}
