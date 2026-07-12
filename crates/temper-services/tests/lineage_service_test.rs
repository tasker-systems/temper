#![cfg(feature = "test-db")]

//! Ledger L2 — `lineage_service::resource_lineage`, access-gated, empirically probed.
//!
//! The fixture is one heterogeneous DAG that exercises every gate and every walk
//! property at once. All edges are `derived_from`; `source derived_from target`
//! (source = deriver/descendant, target = ancestor):
//!
//! ```text
//!   A ─df→ B ─df→ C     B-edge is leads_to, C-edge is express  (BOTH edge_kinds)
//!   A ─df→ F           the A→F edge is FOLDED (must still show, flagged)
//!   A ─df→ H           H is homed where the caller can't read  (endpoint gate)
//!   A ─df→ D           the A→D EDGE is homed where the caller can't read (home gate)
//!   C ─df→ A           a cycle back to the seed
//! ```
//!
//! `p_in` sees `ctx_in`; `p_out` sees only `ctx_out`. The two verified truths:
//! a code-only read of an access predicate finds nothing — so we probe both a
//! readable and an unreadable caller against real rows.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use temper_services::services::lineage_service;

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn personal_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A resource homed in `ctx`, owned by `owner` — visible to `owner` via the
/// personal/home arm of `resources_visible_to`.
async fn resource(pool: &PgPool, title: &str, ctx: Uuid, owner: Uuid) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) \
         VALUES (uuid_generate_v7(), $1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// One dummy event to satisfy the `kb_edges` NOT NULL event FKs.
async fn seed_event(pool: &PgPool, owner: Uuid) -> Uuid {
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (id, profile_id, name) \
         VALUES (uuid_generate_v7(), $1, 'emitter') RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_events (id, event_type_id, emitter_entity_id, payload) \
         SELECT uuid_generate_v7(), et.id, $1, '{}'::jsonb \
           FROM kb_event_types et WHERE et.name = 'resource_created' LIMIT 1 RETURNING id",
    )
    .bind(entity)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn derived_from(
    pool: &PgPool,
    source: Uuid,
    target: Uuid,
    edge_kind: &str,
    polarity: &str,
    home_ctx: Uuid,
    event: Uuid,
    folded: bool,
) {
    sqlx::query(
        "INSERT INTO kb_edges \
           (id, source_table, source_id, target_table, target_id, edge_kind, polarity, label, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, 'kb_resources', $2, $3::edge_kind, \
                 $4::edge_polarity, 'derived_from', 'kb_contexts', $5, $6, $6, $7)",
    )
    .bind(source)
    .bind(target)
    .bind(edge_kind)
    .bind(polarity)
    .bind(home_ctx)
    .bind(event)
    .bind(folded)
    .execute(pool)
    .await
    .unwrap();
}

struct Fixture {
    p_in: Uuid,
    p_out: Uuid,
    a: Uuid,
    b: Uuid,
    c: Uuid,
}

async fn build(pool: &PgPool) -> Fixture {
    let p_in = profile(pool, "lin-in").await;
    let p_out = profile(pool, "lin-out").await;
    let ctx_in = personal_context(pool, p_in, "ctx-in").await;
    let ctx_out = personal_context(pool, p_out, "ctx-out").await;
    let ev = seed_event(pool, p_in).await;

    let a = resource(pool, "Resource A", ctx_in, p_in).await;
    let b = resource(pool, "Resource B", ctx_in, p_in).await;
    let c = resource(pool, "Resource C", ctx_in, p_in).await;
    let f = resource(pool, "Resource F", ctx_in, p_in).await; // reached via a folded edge
    let d = resource(pool, "Resource D", ctx_in, p_in).await; // readable, but its edge is hidden
    let h = resource(pool, "Resource H", ctx_out, p_out).await; // unreadable endpoint

    // A df B (leads_to) ; B df C (express) — both edge_kinds on the ancestor chain
    derived_from(pool, a, b, "leads_to", "inverse", ctx_in, ev, false).await;
    derived_from(pool, b, c, "express", "forward", ctx_in, ev, false).await;
    // A df F — folded edge, still visible
    derived_from(pool, a, f, "leads_to", "inverse", ctx_in, ev, true).await;
    // A df H — endpoint H unreadable → excluded
    derived_from(pool, a, h, "leads_to", "inverse", ctx_in, ev, false).await;
    // A df D — edge homed in ctx_out (unreadable) though D is readable → excluded
    derived_from(pool, a, d, "leads_to", "inverse", ctx_out, ev, false).await;
    // C df A — cycle
    derived_from(pool, c, a, "leads_to", "inverse", ctx_in, ev, false).await;

    Fixture { p_in, p_out, a, b, c }
}

/// title -> (depth, edge_folded)
fn by_title(nodes: &[temper_core::types::lineage::LineageNode]) -> HashMap<String, (i32, bool)> {
    nodes
        .iter()
        .map(|n| (n.title.clone(), (n.depth, n.edge_folded)))
        .collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn ancestors_walk_both_kinds_gated_folded_and_cycle_safe(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    let lineage = lineage_service::resource_lineage(&pool, fx.p_in, fx.a, 16)
        .await
        .expect("A is visible to p_in");

    let anc = by_title(&lineage.ancestors);

    // B reached at depth 1 via the leads_to edge; C at depth 2 via the express
    // edge — proves the walk keys on the LABEL, not edge_kind.
    assert_eq!(anc.get("Resource B"), Some(&(1, false)), "B @1 (leads_to)");
    assert_eq!(anc.get("Resource C"), Some(&(2, false)), "C @2 (express, transitive)");
    // F reached at depth 1, and its edge is folded — a superseded ancestor is
    // shown, flagged.
    assert_eq!(anc.get("Resource F"), Some(&(1, true)), "F @1, folded flag set");
    // Endpoint gate: H is homed where p_in can't read → excluded.
    assert!(!anc.contains_key("Resource H"), "H excluded (endpoint gate)");
    // Home gate: the A→D edge is homed in an unreadable context → excluded, even
    // though D itself is readable.
    assert!(!anc.contains_key("Resource D"), "D excluded (edge-home gate)");
    // Cycle-safety: the seed A is never re-emitted.
    assert!(!anc.contains_key("Resource A"), "seed not re-emitted (cycle safe)");
    assert_eq!(anc.len(), 3, "exactly B, C, F");

    // The ref is decorated and paste-able.
    let b_node = lineage.ancestors.iter().find(|n| n.title == "Resource B").unwrap();
    assert!(b_node.r#ref.ends_with(&fx.b.to_string()), "ref carries the uuid");
    assert!(b_node.r#ref.starts_with("resource-b-"), "ref carries the slug");

    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn descendants_walk_reverse(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    let lineage = lineage_service::resource_lineage(&pool, fx.p_in, fx.c, 16)
        .await
        .expect("C is visible to p_in");

    // Who derives from C? B (B df C) at depth 1, then A (A df B) at depth 2.
    let desc = by_title(&lineage.descendants);
    assert_eq!(desc.get("Resource B"), Some(&(1, false)), "B @1");
    assert_eq!(desc.get("Resource A"), Some(&(2, false)), "A @2");
    assert_eq!(desc.len(), 2, "exactly B, A");

    // And C's own ancestors follow the cycle edge C df A → A, then A df B → B.
    let anc = by_title(&lineage.ancestors);
    assert_eq!(anc.get("Resource A"), Some(&(1, false)), "A @1 via cycle edge");
    assert!(anc.contains_key("Resource B"), "B reachable past the cycle");

    Ok(())
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unreadable_seed_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let fx = build(&pool).await;

    // A is homed in p_in's context; p_out cannot see it at all.
    let err = lineage_service::resource_lineage(&pool, fx.p_out, fx.a, 16)
        .await
        .expect_err("A is invisible to p_out");
    assert!(
        matches!(err, temper_services::error::ApiError::NotFound),
        "invisible seed → NotFound, not an empty leak"
    );

    Ok(())
}
