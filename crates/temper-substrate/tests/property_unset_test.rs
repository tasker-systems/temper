#![cfg(feature = "artifact-tests")]
//! `property_unset` — the key-grain delete verb behind an explicit `null` value in an
//! update's `open_meta`.
//!
//! The projector folds every live row for `(owner, property_key)` — the same predicate
//! `_project_property_set` folds under, minus the insert — and rebuilds the resource's FTS
//! vector for the indexed open keys, because folding the last `tags` row must not leave a
//! stale search vector behind. The payload carries `(owner, key)`, so replay re-folds the
//! SAME key's live set; the `NOT is_folded` floor makes a re-application a zero-row no-op.
//!
//! ONNX-dependent, like every artifact test. Isolated ephemeral DB via
//! `temper_substrate::MIGRATOR`.

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::{EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::replay;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

// Local fixture helpers — duplicated per file rather than shared, the established convention
// (see replay_roundtrip.rs's own header note).

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
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
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn ctx(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    slug: &str,
) -> temper_substrate::ids::ContextId {
    temper_substrate::ids::ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: temper_substrate::ids::ContextId,
    title: &str,
    uri: &str,
    properties: &[(String, serde_json::Value)],
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: uri,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

async fn live_rows_for_key(pool: &sqlx::PgPool, resource: ResourceId, key: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND property_key = $2 AND NOT is_folded",
    )
    .bind(resource.uuid())
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Does the stored vector match a query term? (`@@ plainto_tsquery` — the search_index.rs idiom).
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

async fn update(
    pool: &sqlx::PgPool,
    resource: ResourceId,
    emitter: EntityId,
    unset_keys: &[String],
) -> Result<(), anyhow::Error> {
    writes::update_resource(
        pool,
        writes::UpdateParams {
            sources: vec![],
            resource,
            body: None,
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys,
            chunks: None,
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unset_folds_the_key_and_leaves_sibling_keys_alone(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "unset").await;
    let r = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "Tagged doc",
        "temper://unset/r",
        &[
            ("tags".to_string(), serde_json::json!(["alpha", "beta"])),
            ("descriptor".to_string(), serde_json::json!("kept")),
        ],
    )
    .await;
    assert_eq!(live_rows_for_key(&pool, r, "tags").await, 1);
    assert_eq!(live_rows_for_key(&pool, r, "descriptor").await, 1);

    update(&pool, r, emitter, &["tags".to_string()])
        .await
        .unwrap();

    assert_eq!(
        live_rows_for_key(&pool, r, "tags").await,
        0,
        "the unset key folds its live set"
    );
    assert_eq!(
        live_rows_for_key(&pool, r, "descriptor").await,
        1,
        "a sibling key is untouched — the verb is key-grain"
    );

    // Set-after-unset re-creates the key fresh (the fold-then-insert shape).
    writes::update_resource(
        &pool,
        writes::UpdateParams {
            sources: vec![],
            resource: r,
            body: None,
            title: None,
            origin_uri: None,
            properties: &[("tags".to_string(), serde_json::json!(["gamma"]))],
            unset_keys: &[],
            chunks: None,
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        live_rows_for_key(&pool, r, "tags").await,
        1,
        "a set after an unset asserts a fresh live row"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unset_of_an_absent_key_succeeds_as_a_noop(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "unset").await;
    let r = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "Untouched",
        "temper://unset/noop",
        &[],
    )
    .await;

    // Delete semantics: an absent (or already-unset) key is an idempotent no-op, never a
    // refusal — the caller can already read the meta, so the no-op discloses nothing.
    update(&pool, r, emitter, &["never_set".to_string()])
        .await
        .unwrap();
    assert_eq!(live_rows_for_key(&pool, r, "never_set").await, 0);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn unsetting_tags_rebuilds_the_search_vector(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "unset").await;
    let r = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "Plain body",
        "temper://unset/fts",
        &[("tags".to_string(), serde_json::json!(["alpharhythm"]))],
    )
    .await;
    assert!(
        index_matches(&pool, r.uuid(), "alpharhythm").await,
        "the tag term is indexed through the set path"
    );

    update(&pool, r, emitter, &["tags".to_string()])
        .await
        .unwrap();

    assert!(
        !index_matches(&pool, r.uuid(), "alpharhythm").await,
        "the fold alone would leave a stale vector — the rebuild must run"
    );
}

/// The full roundtrip: set a key, unset it, snapshot, reset to a clean UN-seeded namespace,
/// replay, and prove the key comes back folded — no duplicate, no resurrection. A missing
/// `EventKind::PropertyUnset` arm would hard-fail the replay walk here ("no projector for
/// event type property_unset"), exactly the `resource_finalized` bug shape.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reprojects_a_property_unset(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = ctx(&pool, owner, "unset-replay").await;
    let r = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "Replayed unset",
        "temper://unset/replay",
        &[("tags".to_string(), serde_json::json!(["keep-out"]))],
    )
    .await;
    update(&pool, r, emitter, &["tags".to_string()])
        .await
        .unwrap();

    let before: (i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM kb_properties \
              WHERE owner_table = 'kb_resources' AND owner_id = $1 \
                AND property_key = 'tags'), \
            (SELECT count(*) FROM kb_properties \
              WHERE owner_table = 'kb_resources' AND owner_id = $1 \
                AND property_key = 'tags' AND NOT is_folded)",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, (1, 0), "one folded row after the unset");

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();

    let after: (i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT count(*) FROM kb_properties \
              WHERE owner_table = 'kb_resources' AND owner_id = $1 \
                AND property_key = 'tags'), \
            (SELECT count(*) FROM kb_properties \
              WHERE owner_table = 'kb_resources' AND owner_id = $1 \
                AND property_key = 'tags' AND NOT is_folded)",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "the same row is re-folded identically — no duplicate, no resurrection"
    );
}
