#![cfg(feature = "artifact-tests")]
//! Search Beat 1 — the stored `kb_resource_search_index` is populated and maintained by the
//! event-sourced projection functions (create / block-edit / title-only update), and backfilled.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

/// The boot-seeded canonical `system` profile + entity.
async fn system_actor(
    pool: &sqlx::PgPool,
) -> (
    temper_substrate::ids::ProfileId,
    temper_substrate::ids::EntityId,
) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (
        temper_substrate::ids::ProfileId::from(profile),
        temper_substrate::ids::EntityId::from(entity),
    )
}

async fn ctx(
    pool: &sqlx::PgPool,
    owner: temper_substrate::ids::ProfileId,
    slug: &str,
) -> temper_substrate::ids::ContextId {
    temper_substrate::ids::ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// Does the stored vector match a query term? (`@@ plainto_tsquery`).
async fn index_matches(pool: &sqlx::PgPool, resource: Uuid, term: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT COALESCE((SELECT search_vector @@ plainto_tsquery('english', $2)
           FROM kb_resource_search_index WHERE resource_id = $1), false)",
    )
    .bind(resource)
    .bind(term)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_populates_index_with_title_and_body(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "Salamander architecture",
            origin_uri: "temper://idx/r",
            body: "the quenching pipeline tempers steel",
            doc_type: "concept",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "salamander").await,
        "title term indexed (weight A)"
    );
    assert!(
        index_matches(&pool, r.uuid(), "quenching").await,
        "body term indexed (weight B)"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "unrelated").await,
        "non-present term does not match"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn body_edit_updates_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "Doc",
            origin_uri: "temper://idx/r",
            body: "original lexeme here",
            doc_type: "concept",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap();
    assert!(
        index_matches(&pool, r.uuid(), "original").await,
        "pre-edit body term present"
    );

    writes::update_resource(
        &pool,
        writes::UpdateParams {
            resource: r,
            body: Some("revised distinctive wording"),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "distinctive").await,
        "new body term indexed after edit"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "original").await,
        "superseded body term gone from index"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn title_only_update_updates_index(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "Aardvark",
            origin_uri: "temper://idx/r",
            body: "stable body",
            doc_type: "concept",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap();

    writes::update_resource(
        &pool,
        writes::UpdateParams {
            resource: r,
            body: None,
            title: Some("Pangolin"),
            origin_uri: None,
            properties: &[],
            chunks: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();

    assert!(
        index_matches(&pool, r.uuid(), "pangolin").await,
        "new title term indexed (title-only update)"
    );
    assert!(
        !index_matches(&pool, r.uuid(), "aardvark").await,
        "old title term gone"
    );
    assert!(
        index_matches(&pool, r.uuid(), "stable").await,
        "body unchanged still indexed"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn backfill_covers_preexisting_rows(pool: sqlx::PgPool) {
    // The migration's backfill is upsert + idempotent; re-running it must (a) cover every active
    // resource and (b) leave already-maintained rows correct. We assert coverage: every active
    // resource has an index row, and a known term matches.
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "idx").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "Backfillable",
            origin_uri: "temper://idx/r",
            body: "corpus content word",
            doc_type: "concept",
            home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap();

    let missing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resources r
          WHERE r.is_active
            AND NOT EXISTS (SELECT 1 FROM kb_resource_search_index si WHERE si.resource_id = r.id)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(missing, 0, "every active resource has an index row");
    assert!(
        index_matches(&pool, r.uuid(), "corpus").await,
        "backfilled/maintained term matches"
    );
}
