#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! Dark-launch coverage for the substrate (`temper_next.*`) edge-read path.
//!
//! Exercises `edge_service::list_resource_edges_next` against the grafted
//! substrate schema. Fixtures seed `temper_next.*` directly with raw queries (no
//! macros → no `.sqlx` cache entries for the test target). Edge visibility goes
//! through `edges_visible_to`, which gates the edge's HOME anchor by team-share,
//! so the fixture shares the owner-scoped context to the owner's personal team.

mod common;

use sqlx::PgPool;
use temper_api::error::ApiError;
use temper_api::services::edge_service;
use uuid::Uuid;

/// Seed a bare substrate profile inside a `temper_next` search_path transaction
/// so the `sync_personal_team` AFTER-INSERT trigger (unqualified body) lands the
/// personal team in `temper_next.kb_teams`. Returns the profile id, its handle,
/// and the personal team id (the team the owner reaches for context-share).
async fn seed_owner(pool: &PgPool, label: &str) -> (Uuid, Uuid) {
    let id = Uuid::now_v7();
    let handle = format!("{label}-{}", &id.simple().to_string()[..8]);

    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await
        .expect("set search_path");
    sqlx::query(
        "INSERT INTO temper_next.kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
    )
    .bind(id)
    .bind(&handle)
    .bind(label)
    .execute(&mut *tx)
    .await
    .expect("seed substrate profile");
    tx.commit().await.expect("commit");

    let team: Uuid = sqlx::query_scalar("SELECT id FROM temper_next.kb_teams WHERE slug = $1")
        .bind(format!("personal-{handle}"))
        .fetch_one(pool)
        .await
        .expect("personal team created by trigger");
    (id, team)
}

/// Insert an owner-scoped context and share it to the owner's personal team
/// (so the edge homed there is anchor-readable). Returns the context id.
async fn seed_shared_context(pool: &PgPool, owner: Uuid, team: Uuid, slug: &str) -> Uuid {
    let ctx = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO temper_next.kb_contexts (id, owner_table, owner_id, slug, name)
         VALUES ($1, 'kb_profiles', $2, $3, $3)",
    )
    .bind(ctx)
    .bind(owner)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed context");
    sqlx::query("INSERT INTO temper_next.kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(ctx)
        .bind(team)
        .execute(pool)
        .await
        .expect("share context to personal team");
    ctx
}

/// Home a fresh substrate resource (owned by `owner`) into `ctx`. Returns its id.
async fn seed_resource(pool: &PgPool, owner: Uuid, ctx: Uuid, title: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(title)
        .bind(format!("temper://test/{id}"))
        .execute(pool)
        .await
        .expect("seed resource");
    sqlx::query(
        "INSERT INTO temper_next.kb_resource_homes
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home resource");
    id
}

/// Seed the minimal event scaffolding (event type + emitter entity + event) the
/// `kb_edges` NOT-NULL FKs require, returning the event id.
async fn seed_event(pool: &PgPool, owner: Uuid) -> Uuid {
    let etype = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_event_types (id, name) VALUES ($1, $2)")
        .bind(etype)
        .bind(format!("test-evt-{}", &etype.simple().to_string()[..8]))
        .execute(pool)
        .await
        .expect("seed event type");
    let entity = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(entity)
        .bind(owner)
        .bind("test-entity")
        .execute(pool)
        .await
        .expect("seed entity");
    let event = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO temper_next.kb_events (id, event_type_id, emitter_entity_id)
         VALUES ($1, $2, $3)",
    )
    .bind(event)
    .bind(etype)
    .bind(entity)
    .execute(pool)
    .await
    .expect("seed event");
    event
}

/// Assert one `source -> target` edge homed in `ctx`, returning its id.
async fn seed_edge(pool: &PgPool, source: Uuid, target: Uuid, ctx: Uuid, event: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO temper_next.kb_edges
           (id, source_table, source_id, target_table, target_id,
            edge_kind, polarity, label, weight,
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, 'kb_resources', $2, 'kb_resources', $3,
                 'contains', 'forward', 'parent_of', 1.0,
                 'kb_contexts', $4, $5, $5, false)",
    )
    .bind(id)
    .bind(source)
    .bind(target)
    .bind(ctx)
    .bind(event)
    .execute(pool)
    .await
    .expect("seed edge");
    id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn lists_visible_edge_with_peer_direction_and_slug(pool: PgPool) {
    let (owner, team) = seed_owner(&pool, "owner").await;
    let ctx = seed_shared_context(&pool, owner, team, "edge-ctx").await;
    let source = seed_resource(&pool, owner, ctx, "Source Doc").await;
    let target = seed_resource(&pool, owner, ctx, "Peer Target Doc").await;
    let event = seed_event(&pool, owner).await;
    let edge = seed_edge(&pool, source, target, ctx, event).await;

    // From the SOURCE endpoint: one outgoing edge to the target.
    let rows = edge_service::list_resource_edges_next(&pool, owner, source)
        .await
        .expect("list_resource_edges_next from source");
    assert_eq!(rows.len(), 1, "exactly the one seeded edge is visible");
    let row = &rows[0];
    assert_eq!(row.edge_id, edge);
    assert_eq!(row.peer_resource_id, target);
    assert_eq!(row.peer_title, "Peer Target Doc");
    assert_eq!(
        row.peer_slug, "peer-target-doc",
        "peer_slug derived from peer title (§7-dissolved)"
    );
    assert_eq!(row.direction, "outgoing");
    assert_eq!(row.label, "parent_of");

    // From the TARGET endpoint: the same edge presents as incoming, peer = source.
    let rows = edge_service::list_resource_edges_next(&pool, owner, target)
        .await
        .expect("list_resource_edges_next from target");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].peer_resource_id, source);
    assert_eq!(rows[0].direction, "incoming");
    assert_eq!(rows[0].peer_slug, "source-doc");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invisible_resource_returns_not_found(pool: PgPool) {
    let (owner, _team) = seed_owner(&pool, "owner").await;

    // A resource id the owner cannot see (never seeded) → NotFound, matching the
    // legacy 404 gate.
    let err = edge_service::list_resource_edges_next(&pool, owner, Uuid::now_v7())
        .await
        .expect_err("invisible resource must be NotFound");
    assert!(
        matches!(err, ApiError::NotFound),
        "expected NotFound, got {err:?}"
    );
}
