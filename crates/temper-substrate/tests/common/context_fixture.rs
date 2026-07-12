//! A real, embedded CONTEXT to materialize regions over — the fixture the telos and two-clock tests
//! share.
//!
//! Extracted from T5's `context_telos_salience.rs` when T6 needed the same construction. Shared rather
//! than copied: these builders encode a handful of non-obvious constraints (fold the seeded facets, or
//! the embedding is no longer the only formation signal; a task carries NO body, because it feeds the
//! CENSUS and never the telos vector; close a task by FOLDING its stage row, because
//! `context_goal_liveness` depends on `NOT is_folded` selecting exactly one). A second copy would
//! drift from those constraints silently.

#![allow(dead_code)]

use sqlx::PgPool;
use uuid::Uuid;

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// An event id to satisfy the NOT NULL FKs on fixture rows.
pub async fn any_event(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("event")
}

/// Re-home a seeded cogmap's corpus into a context and fold its facets.
///
/// Both halves matter. A production context homes its resources under `kb_contexts`, and it carries
/// **no facets** — so leaving the seed's facets live would hand formation a declared signal that no
/// real context has, and the test would no longer be exercising the regime it claims to.
pub async fn rehome_corpus_into_context(pool: &PgPool, ctx: Uuid, cogmap: Uuid) {
    sqlx::query(
        "UPDATE kb_resource_homes SET anchor_table='kb_contexts', anchor_id=$1 \
         WHERE anchor_table='kb_cogmaps' AND anchor_id=$2",
    )
    .bind(ctx)
    .bind(cogmap)
    .execute(pool)
    .await
    .expect("re-home");
    sqlx::query(
        "UPDATE kb_properties SET is_folded=true \
         WHERE owner_table='kb_resources' AND property_key='facet'",
    )
    .execute(pool)
    .await
    .expect("fold facets");
}

/// A goal resource homed in `ctx`, carrying one chunk with the **given** synthetic embedding.
///
/// The vector is supplied rather than embedded so two goals can be made deliberately DISSIMILAR
/// (near-orthogonal). If both goals embedded to roughly the same place — which real prose about one
/// project tends to — the telos would barely rotate when one is closed, and a test could pass while
/// proving nothing. Handing the vectors in makes the telos shift a controlled input.
pub async fn seed_goal(pool: &PgPool, ctx: Uuid, owner: Uuid, title: &str, v: Vec<f32>) -> Uuid {
    let ev = any_event(pool).await;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,'') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("goal");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
           originator_profile_id, owner_profile_id) VALUES ($1,'kb_contexts',$2,$3,$3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home");
    set_prop(pool, id, "doc_type", "goal").await;

    let block: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, 0, $2, $2) RETURNING id",
    )
    .bind(id)
    .bind(ev)
    .fetch_one(pool)
    .await
    .expect("block");
    sqlx::query(
        "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding) \
         VALUES ($1, $2, 0, $3, $4::vector)",
    )
    .bind(block)
    .bind(id)
    .bind(sha256_hex(title))
    .bind(format!(
        "[{}]",
        v.iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
            .join(",")
    ))
    .execute(pool)
    .await
    .expect("chunk");
    id
}

/// A task homed in `ctx` that `advances` `goal`, at the given stage. No body — a task contributes to
/// the CENSUS, never to the telos vector.
pub async fn seed_task(pool: &PgPool, ctx: Uuid, owner: Uuid, goal: Uuid, stage: &str) -> Uuid {
    let ev = any_event(pool).await;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,'') RETURNING id",
    )
    .bind(format!("task {stage}"))
    .fetch_one(pool)
    .await
    .expect("task");
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, \
           originator_profile_id, owner_profile_id) VALUES ($1,'kb_contexts',$2,$3,$3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home");
    set_prop(pool, id, "doc_type", "task").await;
    set_prop(pool, id, "temper-stage", stage).await;
    sqlx::query(
        "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, \
           polarity, label, home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources',$1,'kb_resources',$2,'leads_to','forward','advances', \
           'kb_contexts',$3,$4,$4)",
    )
    .bind(id)
    .bind(goal)
    .bind(ctx)
    .bind(ev)
    .execute(pool)
    .await
    .expect("advances");
    id
}

pub async fn set_prop(pool: &PgPool, owner: Uuid, key: &str, value: &str) {
    let ev = any_event(pool).await;
    sqlx::query(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, \
           asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources',$1,$2,to_jsonb($3::text),$4,$4)",
    )
    .bind(owner)
    .bind(key)
    .bind(value)
    .bind(ev)
    .execute(pool)
    .await
    .expect("prop");
}

/// Close a task the way the real path does: FOLD the live stage row, assert a new one. (A blind
/// UPDATE would work here too, but folding is what production does, and `context_goal_liveness`
/// depends on `NOT is_folded` selecting exactly one row.)
pub async fn close_task(pool: &PgPool, task: Uuid) {
    sqlx::query(
        "UPDATE kb_properties SET is_folded = true \
         WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key='temper-stage'",
    )
    .bind(task)
    .execute(pool)
    .await
    .expect("fold stage");
    set_prop(pool, task, "temper-stage", "done").await;
}

/// (region id, salience, telos_alignment) for every live region of this context, id-ordered.
pub async fn readouts(pool: &PgPool, ctx: Uuid) -> Vec<(Uuid, f64, Option<f64>)> {
    sqlx::query_as(
        "SELECT id, salience, telos_alignment FROM kb_cogmap_regions \
         WHERE home_anchor_table='kb_contexts' AND home_anchor_id=$1 AND NOT is_folded \
         ORDER BY id",
    )
    .bind(ctx)
    .fetch_all(pool)
    .await
    .expect("readouts")
}

pub async fn telos_centroid(pool: &PgPool, ctx: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT telos_centroid::text FROM kb_contexts WHERE id=$1")
        .bind(ctx)
        .fetch_one(pool)
        .await
        .expect("telos_centroid")
}
