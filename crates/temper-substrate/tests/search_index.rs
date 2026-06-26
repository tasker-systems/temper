#![cfg(feature = "artifact-tests")]
//! Search Beat 1 — the stored `kb_resource_search_index` is populated and maintained by the
//! event-sourced projection functions (create / block-edit / title-only update), and backfilled.
//! Isolated ephemeral DB via `MIGRATOR`.

mod common;

use temper_substrate::readback;
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
    // The migration's backfill (20260626000001) runs after 20260625000001, which seeds the L0 kernel
    // telos resource. This test asserts that both the pre-existing telos resource AND a freshly
    // created resource have index rows — i.e. the backfill covered all active resources, not just
    // those created after the migration ran.
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

/// Soft-deleted resources are excluded from `fts_search` via the `WHERE r.is_active` clause added
/// in Beat 1. The index ROW persists (the delete-trigger does not remove it); the active-resource
/// JOIN filters it out at read time.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn soft_deleted_resource_excluded_from_fts_search(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "del").await;
    let r = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "Phlogiston study",
            origin_uri: "temper://del/r",
            body: "phlogiston theory explains combustion",
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

    // Pre-delete: resource is found by FTS.
    let before = readback::fts_search(&pool, owner.uuid(), "phlogiston")
        .await
        .unwrap();
    assert!(
        before.contains(&r.uuid()),
        "resource must appear in FTS results before soft-delete"
    );

    // Soft-delete via writes surface (sets kb_resources.is_active = false).
    writes::delete_resource(&pool, r, emitter).await.unwrap();

    // Post-delete: WHERE r.is_active filters the resource out of FTS results.
    let after = readback::fts_search(&pool, owner.uuid(), "phlogiston")
        .await
        .unwrap();
    assert!(
        !after.contains(&r.uuid()),
        "soft-deleted resource must be absent from FTS results (WHERE r.is_active)"
    );

    // The index row persists — delete is soft, not cascaded to the search index.
    // The filter happens at read time, not at delete time (stale-row-is-harmless property).
    let index_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_search_index WHERE resource_id = $1")
            .bind(r.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        index_count, 1,
        "index row persists after soft-delete (filtered at read time, not deleted)"
    );
}

/// The stored-index `fts_search` returns the SAME id set as the legacy inline build for title/body
/// terms. The inline query below is the pre-swap recipe verbatim (the behavior-preservation gate).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn fts_search_parity_with_inline_recipe(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "p").await;
    for (t, b, u) in [
        (
            "Quenching furnace",
            "tempering steel at temperature",
            "temper://p/1",
        ),
        (
            "Annealing notes",
            "slow cooling relieves stress",
            "temper://p/2",
        ),
        ("Unrelated doc", "nothing about metal here", "temper://p/3"),
    ] {
        writes::create_resource(
            &pool,
            writes::CreateParams {
                title: t,
                origin_uri: u,
                body: b,
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
    }

    // Legacy inline recipe (pre-swap), inlined here as the parity oracle.
    async fn inline_fts(pool: &sqlx::PgPool, principal: Uuid, q: &str) -> Vec<Uuid> {
        let rows = sqlx::query(
            "WITH doc AS (
               SELECT r.id,
                      setweight(to_tsvector('english', r.title), 'A') ||
                      setweight(to_tsvector('english', COALESCE(string_agg(cc.content, ' '), '')), 'B')
                        AS search_vector
                 FROM kb_resources r
                 JOIN resources_visible_to($1) v ON v.resource_id = r.id
                 LEFT JOIN kb_chunks c ON c.resource_id = r.id AND c.is_current
                 LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
                GROUP BY r.id, r.title)
             SELECT id FROM doc
              WHERE search_vector @@ plainto_tsquery('english', $2)
              ORDER BY ts_rank(search_vector, plainto_tsquery('english', $2)) DESC")
            .bind(principal).bind(q).fetch_all(pool).await.unwrap();
        rows.iter()
            .map(|r| sqlx::Row::get::<Uuid, _>(r, "id"))
            .collect()
    }

    for q in [
        "tempering",
        "cooling",
        "metal",
        "quenching steel",
        "furnace",
    ] {
        let mut want = inline_fts(&pool, owner.uuid(), q).await;
        let mut got = readback::fts_search(&pool, owner.uuid(), q).await.unwrap();
        want.sort();
        got.sort();
        assert_eq!(
            got, want,
            "stored-index fts_search set parity for query {q:?}"
        );
    }
}
