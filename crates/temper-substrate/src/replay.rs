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

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
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
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_content_blocks t",
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
    "kb_topics",
    "kb_event_types",
    "kb_events",
];

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
          WHERE et.name IN ('cogmap_seeded','resource_created','block_mutated') ORDER BY e.id",
    )
    .fetch_all(pool)
    .await?;
    for r in rows {
        let event_id: Uuid = r.get(0);
        let name: String = r.get(1);
        let payload: serde_json::Value = r.get(2);
        let manifests = match name.as_str() {
            "cogmap_seeded" => payload.pointer("/telos/blocks").cloned(),
            "resource_created" => payload.get("blocks").cloned(),
            // block_mutated carries a flat `chunks` array (one block); wrap it as a single
            // pseudo-block so the chunk-extraction loop below stays uniform.
            "block_mutated" => payload
                .get("chunks")
                .cloned()
                .map(|chunks| serde_json::json!([{ "chunks": chunks }])),
            _ => None,
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
                    "SELECT cc.content, c.embedding::text \
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
                side.insert(
                    chunk_id.to_string(),
                    serde_json::json!({ "content": content, "embedding": embedding }),
                );
            }
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
        match name.as_str() {
            "cogmap_seeded" => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_cogmap_seeded($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            "resource_created" => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_resource_created($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            "block_mutated" => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_block_mutated($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
            "relationship_asserted" => {
                sqlx::query("SELECT _project_relationship_asserted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "property_asserted" => {
                sqlx::query("SELECT _project_property_asserted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "property_set" => {
                sqlx::query("SELECT _project_property_set($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "lens_created" => {
                sqlx::query("SELECT _project_lens_created($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "region_materialized" => {
                sqlx::query("SELECT _project_region_materialized($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            // WS6 4c mutations + the relationship_folded sibling (payload-only projectors, no sidecar).
            "relationship_folded" => {
                sqlx::query("SELECT _project_relationship_folded($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "relationship_retyped" => {
                sqlx::query("SELECT _project_relationship_retyped($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "relationship_reweighted" => {
                sqlx::query("SELECT _project_relationship_reweighted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "resource_deleted" => {
                sqlx::query("SELECT _project_resource_deleted($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "resource_updated" => {
                sqlx::query("SELECT _project_resource_updated($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "resource_rehomed" => {
                sqlx::query("SELECT _project_resource_rehomed($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "delegated_launch" => {
                sqlx::query("SELECT _project_delegated_launch($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "invocation_closed" => {
                sqlx::query("SELECT _project_invocation_closed($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            other => anyhow::bail!("replay: no projector for event type {other}"),
        }
    }
    restore_table(pool, "kb_team_cogmaps", &snap.team_cogmaps).await?;
    Ok(())
}

/// The recorded materialization acts (last per lens):
/// (cogmap_id, lens_id, watermark_event_id, membership_fingerprint) — for the region re-proof.
pub async fn recorded_materializations(pool: &PgPool) -> Result<Vec<(Uuid, Uuid, Uuid, String)>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (e.payload->>'lens_id') \
                (e.payload->>'cogmap_id')::uuid, (e.payload->>'lens_id')::uuid, \
                (e.payload->>'watermark_event_id')::uuid, \
                e.payload->>'membership_fingerprint' \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'region_materialized' \
          ORDER BY e.payload->>'lens_id', e.id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
        .collect())
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
    "cogmap_seeded",
    "relationship_asserted",
    "relationship_retyped",
    "relationship_reweighted",
    "relationship_folded",
    "relationship_decayed",
    "relationship_corrected",
    "property_asserted",
];

/// Did any event whose type is in `names` touch this cogmap after `watermark`? The shared body behind
/// the formation and content gates — the anchor-scoping predicate is load-bearing and easy to get
/// wrong, so it lives in exactly one place.
async fn touched_since(
    pool: &PgPool,
    cogmap: Uuid,
    watermark: Uuid,
    names: &[&str],
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS ( \
            SELECT 1 FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
             WHERE e.id > $2 \
               AND e.producing_anchor_table = 'kb_cogmaps' AND e.producing_anchor_id = $1 \
               AND et.name = ANY($3))",
    )
    .bind(cogmap)
    .bind(watermark)
    .bind(names)
    .fetch_one(pool)
    .await?)
}

/// True iff a FORMATION-affecting event (structural ∪ content — the region-formation inputs) touched
/// the cogmap after the given watermark. A recorded fingerprint is only re-provable when this is
/// false — otherwise the recorded act is legitimately stale relative to the substrate (the
/// drift-detection concept), and re-materialization is expected to differ.
pub async fn formation_touched_since(pool: &PgPool, cogmap: Uuid, watermark: Uuid) -> Result<bool> {
    let names: Vec<&str> = STRUCTURAL_EVENTS
        .iter()
        .chain(CONTENT_EVENTS)
        .copied()
        .collect();
    touched_since(pool, cogmap, watermark, &names).await
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
    cogmap: Uuid,
    watermark: Uuid,
) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT b.resource_id \
           FROM kb_events e \
           JOIN kb_event_types et ON et.id = e.event_type_id \
           JOIN kb_content_blocks b ON b.id = (e.payload->>'block_id')::uuid \
          WHERE e.id > $2 \
            AND e.producing_anchor_table = 'kb_cogmaps' AND e.producing_anchor_id = $1 \
            AND et.name = ANY($3)",
    )
    .bind(cogmap)
    .bind(watermark)
    .bind(CONTENT_EVENTS)
    .fetch_all(pool)
    .await?)
}
