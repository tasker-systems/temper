#![cfg(feature = "artifact-tests")]
//! Search Beat 2 — Surface A candidate functions + the unified blend, on the substrate.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool).await.unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile).fetch_one(pool).await.unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn ctx(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug).await.unwrap())
}

/// Create a body-only `concept` resource (no chunks needed for FTS — body is indexed by Beat 1).
async fn mk(pool: &sqlx::PgPool, home: ContextId, owner: ProfileId, emitter: EntityId,
            title: &str, body: &str, uri: &str) -> Uuid {
    writes::create_resource(pool, writes::CreateParams {
        title, origin_uri: uri, body, doc_type: "concept",
        home, owner, originator: owner, emitter, properties: &[], chunks: None,
    }).await.unwrap().uuid()
}

/// Rows from `search_fts_candidates`, as (id, fts_norm).
async fn fts_candidates(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<(Uuid, f32)> {
    use sqlx::Row;
    sqlx::query("SELECT resource_id, fts_norm FROM search_fts_candidates($1, $2)")
        .bind(principal).bind(q).fetch_all(pool).await.unwrap()
        .iter().map(|r| (r.get::<Uuid, _>("resource_id"), r.get::<f32, _>("fts_norm"))).collect()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_candidates_normalized_and_scoped(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "fts").await;
    let hit = mk(&pool, home, owner, emitter, "Quenching furnace", "tempering steel hot", "temper://fts/1").await;
    let _miss = mk(&pool, home, owner, emitter, "Unrelated", "nothing relevant here", "temper://fts/2").await;

    let got = fts_candidates(&pool, owner.uuid(), "tempering").await;
    let ids: Vec<Uuid> = got.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&hit), "matching resource is a candidate");
    assert!(!ids.contains(&_miss), "non-matching resource is absent");
    let score = got.iter().find(|(id, _)| *id == hit).unwrap().1;
    assert!(score > 0.0 && score < 1.0, "ts_rank|32 normalizes into [0,1): got {score}");

    // Empty query → zero rows (term-zero path).
    assert!(fts_candidates(&pool, owner.uuid(), "").await.is_empty(), "empty query yields no candidates");
}
