//! SQL-level semantics for the R5 element-trail functions
//! (`element_trail_edge`, `element_trail_node`). Proves the keying is grounded
//! in the emitter code — NOT `correlation_id` for edges, and gated via
//! `anchor_readable_by_profile` (not `edges_visible_to`, which would hide a
//! folded edge's own trail) — before the HTTP endpoint is exercised (that is
//! `element_trail_e2e.rs`).
#![cfg(feature = "test-db")]

mod common;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

type TrailRow = (Uuid, String, Uuid, DateTime<Utc>, Value);

/// Insert a profile with the given handle, return its id.
async fn mk_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("insert profile")
}

/// Insert an authoring entity for the given profile.
async fn mk_entity(pool: &sqlx::PgPool, profile: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id")
        .bind(profile)
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("insert entity")
}

/// A `kb_contexts` row owned directly by `profile` — readable via
/// `anchor_readable_by_profile`'s personal-context clause.
async fn mk_owned_context(pool: &sqlx::PgPool, profile: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(profile)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("insert context")
}

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

/// Home a resource to a `kb_contexts` anchor, owned/originated by `profile` —
/// the branch `resources_visible_to` reads for direct ownership.
async fn home_resource(pool: &sqlx::PgPool, resource: Uuid, anchor_context: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(anchor_context)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home resource");
}

/// Insert a raw `kb_events` row of the given canonical type, keyed by whatever
/// the caller puts in `payload`. Bypasses the `_event_append`/projector SQL
/// entry points on purpose — this test proves what `element_trail_*` reads out
/// of the ledger, independent of which write path produced it.
async fn insert_event(
    pool: &sqlx::PgPool,
    type_name: &str,
    emitter: Uuid,
    payload: Value,
    metadata: Value,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, payload, metadata) \
         VALUES ((SELECT id FROM kb_event_types WHERE name = $1), $2, $3, $4) RETURNING id",
    )
    .bind(type_name)
    .bind(emitter)
    .bind(payload)
    .bind(metadata)
    .fetch_one(pool)
    .await
    .expect("insert event")
}

#[allow(clippy::too_many_arguments)]
async fn insert_edge(
    pool: &sqlx::PgPool,
    id: Uuid,
    home_anchor_table: &str,
    home_anchor_id: Uuid,
    asserted_by_event_id: Uuid,
    is_folded: bool,
) {
    sqlx::query(
        "INSERT INTO kb_edges \
             (id, source_table, source_id, target_table, target_id, edge_kind, polarity, weight, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded) \
         VALUES ($1, 'kb_resources', $2, 'kb_resources', $3, 'contains', 'forward', 1.0, \
                 $4, $5, $6, $6, $7)",
    )
    .bind(id)
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .bind(home_anchor_table)
    .bind(home_anchor_id)
    .bind(asserted_by_event_id)
    .bind(is_folded)
    .execute(pool)
    .await
    .expect("insert edge");
}

async fn insert_content_block(
    pool: &sqlx::PgPool,
    resource: Uuid,
    seq: i32,
    genesis_event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(resource)
    .bind(seq)
    .bind(genesis_event)
    .fetch_one(pool)
    .await
    .expect("insert content block")
}

async fn element_trail_edge(pool: &sqlx::PgPool, profile: Uuid, edge: Uuid) -> Vec<TrailRow> {
    sqlx::query_as::<_, TrailRow>(
        "SELECT event_id, kind, actor_entity_id, occurred_at, metadata \
         FROM element_trail_edge($1, $2)",
    )
    .bind(profile)
    .bind(edge)
    .fetch_all(pool)
    .await
    .expect("element_trail_edge")
}

async fn element_trail_node(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> Vec<TrailRow> {
    sqlx::query_as::<_, TrailRow>(
        "SELECT event_id, kind, actor_entity_id, occurred_at, metadata \
         FROM element_trail_node($1, $2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("element_trail_node")
}

/// Edge trail: an assert + a later reweight event share the same
/// `payload->>'edge_id'` (NOT a shared `correlation_id` — that's the bug this
/// test guards against). Both come back ordered by `e.id`. Folding the edge
/// does NOT hide its trail — proves the gate is `anchor_readable_by_profile`
/// on the edge's home anchor, not `edges_visible_to` (which filters `NOT
/// is_folded`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edge_trail_keys_on_edge_id_and_survives_fold(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "ett-tester").await;
    let entity = mk_entity(&pool, profile, "ett-entity").await;
    let context = mk_owned_context(&pool, profile, "ett-context").await;

    let edge_id = Uuid::now_v7();

    let assert_event = insert_event(
        &pool,
        "relationship_asserted",
        entity,
        json!({"edge_id": edge_id, "weight": 1.0}),
        json!({}),
    )
    .await;
    insert_edge(&pool, edge_id, "kb_contexts", context, assert_event, false).await;

    let reweight_event = insert_event(
        &pool,
        "relationship_reweighted",
        entity,
        json!({"edge_id": edge_id, "weight": 2.0}),
        json!({}),
    )
    .await;

    let rows = element_trail_edge(&pool, profile, edge_id).await;
    assert_eq!(
        rows.len(),
        2,
        "both events keyed by the shared edge_id surface: {rows:?}"
    );
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![assert_event, reweight_event],
        "ordered by e.id (emission order): {rows:?}"
    );
    assert_eq!(rows[0].1, "relationship_asserted");
    assert_eq!(rows[1].1, "relationship_reweighted");
    assert!(rows.iter().all(|r| r.2 == entity));

    // Fold the edge — the trail must STILL be returned (proves the gate is the
    // home-anchor read check, not edges_visible_to's NOT is_folded filter).
    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE id = $1")
        .bind(edge_id)
        .execute(&pool)
        .await
        .expect("fold edge");

    let rows_after_fold = element_trail_edge(&pool, profile, edge_id).await;
    assert_eq!(
        rows_after_fold.len(),
        2,
        "a folded edge's trail is still visible: {rows_after_fold:?}"
    );
}

/// Node trail: a `resource_created` event (keyed by `resource_id`), a
/// `property_set` event (keyed by `owner.id` + `owner.table = 'kb_resources'`),
/// and a `block_mutated` event (keyed only by `block_id`, attributed via
/// `kb_content_blocks.resource_id`) all surface through the union — gated once
/// via `resources_visible_to`. A resource not visible to the caller returns no
/// trail at all, even though it has its own `resource_created` event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn node_trail_unions_three_key_shapes_and_gates_by_visibility(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "ntt-tester").await;
    let entity = mk_entity(&pool, profile, "ntt-entity").await;
    let context = mk_owned_context(&pool, profile, "ntt-context").await;

    let visible = create_resource(&pool, "ntt visible", "temper://ntt/visible").await;
    home_resource(&pool, visible, context, profile).await;

    let resource_created = insert_event(
        &pool,
        "resource_created",
        entity,
        json!({"resource_id": visible}),
        json!({}),
    )
    .await;

    let property_set = insert_event(
        &pool,
        "property_set",
        entity,
        json!({"owner": {"table": "kb_resources", "id": visible}, "property_key": "doc_type"}),
        json!({}),
    )
    .await;

    let block = insert_content_block(&pool, visible, 0, resource_created).await;
    let block_mutated = insert_event(
        &pool,
        "block_mutated",
        entity,
        json!({"block_id": block}),
        json!({}),
    )
    .await;

    let rows = element_trail_node(&pool, profile, visible).await;
    let ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    assert_eq!(
        rows.len(),
        3,
        "all three key-shapes surface via the union: {rows:?}"
    );
    assert!(ids.contains(&resource_created), "resource_id key: {rows:?}");
    assert!(
        ids.contains(&property_set),
        "owner.id + owner.table key: {rows:?}"
    );
    assert!(
        ids.contains(&block_mutated),
        "block_id -> kb_content_blocks key: {rows:?}"
    );
    // ordered by e.id
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "ordered by e.id: {rows:?}");

    // A resource NOT visible to `profile` (no home tying it to them) — even
    // though it has its own resource_created event, the trail comes back
    // empty because the visibility gate checks p_resource itself.
    let not_visible = create_resource(&pool, "ntt not visible", "temper://ntt/nv").await;
    insert_event(
        &pool,
        "resource_created",
        entity,
        json!({"resource_id": not_visible}),
        json!({}),
    )
    .await;

    let rows_not_visible = element_trail_node(&pool, profile, not_visible).await;
    assert!(
        rows_not_visible.is_empty(),
        "a resource not visible to the caller yields no trail: {rows_not_visible:?}"
    );
}
