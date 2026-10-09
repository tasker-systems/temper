#![cfg(feature = "test-db")]
//! Field-grain scrub witnesses (spec 2026-10-09 field-grain-scrub).
//!
//! `_project_property_unset` (migration 20261018100000) is the one body of the `property_unset`
//! projection: the write guard, the fold of `(kb_resources, owner, key)`'s live rows, and the
//! search-vector rebuild for `keywords` / `descriptor` / `tags`. The fire path and replay both
//! reach it through `events::project_property_unset`; a SQL act clearing a property family
//! (S4 step 3) can call it directly. These witnesses call it directly, against a world built
//! through the real write paths.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, ProfileId, ResourceId};
use temper_substrate::payloads::{self, AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

/// The indexed term the `tags` witnesses look for in the search vector.
const TAG_TERM: &str = "alpharhythm";

fn chunk_hash(prose: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(prose.trim()))
}

fn chunk(prose: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash(prose),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: "first".to_string(),
        heading_depth: 1,
    }
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

async fn make_home(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn setup(pool: &PgPool) -> (ProfileId, EntityId, ContextId) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = make_home(pool, owner, "field-scrub").await;
    (owner, emitter, home)
}

/// A complete resource with one block and `properties`, through the create path.
async fn create(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    slug: &str,
    properties: &[(String, serde_json::Value)],
) -> ResourceId {
    let origin = format!("test://{slug}");
    let body = "a plain body";
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
            body,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties,
            chunks: Some(vec![chunk(body)]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create through the create path")
}

/// The `property_unset` payload the fire path writes for `(resource, key)`.
fn unset_payload(resource: ResourceId, key: &str) -> serde_json::Value {
    serde_json::to_value(payloads::PropertyUnset {
        owner: AnchorRef::resource(resource),
        property_key: key.to_owned(),
    })
    .unwrap()
}

/// Append a `property_unset` event for `(resource, key)` as the fire path does, with no
/// projection, and return its id and payload: the witnesses then project it themselves.
async fn append_unset(
    pool: &PgPool,
    emitter: EntityId,
    resource: ResourceId,
    key: &str,
) -> (Uuid, serde_json::Value) {
    let payload = unset_payload(resource, key);
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('property_unset', $1, a.anchor_table, a.anchor_id, $2) \
           FROM _property_owner_anchor('kb_resources', $3) a",
    )
    .bind(emitter)
    .bind(&payload)
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .expect("the property_unset event appends");
    (event, payload)
}

/// The projector, called directly.
async fn project_unset(
    pool: &PgPool,
    event: Uuid,
    payload: &serde_json::Value,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT _project_property_unset($1, $2)")
        .bind(event)
        .bind(payload)
        .fetch_one(pool)
        .await
}

async fn live_rows(pool: &PgPool, owner_table: &str, owner_id: Uuid, key: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = $1 AND owner_id = $2 AND property_key = $3 AND NOT is_folded",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn search_vector(pool: &PgPool, resource: ResourceId) -> Option<String> {
    sqlx::query_scalar(
        "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(resource.uuid())
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn vector_matches(pool: &PgPool, resource: ResourceId, term: &str) -> bool {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT search_vector @@ plainto_tsquery('english', $2) \
           FROM kb_resource_search_index WHERE resource_id = $1), false)",
    )
    .bind(resource.uuid())
    .bind(term)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The erasure act, as the system operator under a fresh request reference; returns the
/// `resource_erased` event id.
async fn erase(pool: &PgPool, resource: ResourceId) -> Uuid {
    let (_, operator_entity) = system_actor(pool).await;
    let raw: String =
        sqlx::query_scalar("SELECT (resource_erasure_execute($1,$2,$3,$4)->>'event_id')::text")
            .bind(resource.uuid())
            .bind(operator_entity)
            .bind(operator_entity)
            .bind(Uuid::now_v7())
            .fetch_one(pool)
            .await
            .expect("the erasure completes");
    Uuid::parse_str(&raw).expect("the event id parses")
}

/// Called directly, `_project_property_unset` folds every live row of `(kb_resources, R, key)`
/// and returns their count. Another resource's row of the same key, an edge-owned row of the
/// same key text, and R's other keys stay live.
///
/// FAILS IF: the function does not exist; the count is not the number of R's live rows of the
/// key; a folded row does not carry the unset event as `last_event_id`; or the fold reaches past
/// `(kb_resources, R, key)`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unset_projector_folds_only_the_owners_live_rows_of_the_key(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let tags = vec![("tags".to_string(), serde_json::json!(["alpha"]))];
    let mut r_props = tags.clone();
    r_props.push(("descriptor".to_string(), serde_json::json!("kept")));
    let r = create(&pool, owner, emitter, home, "scrub-r", &r_props).await;
    let twin = create(&pool, owner, emitter, home, "scrub-twin", &tags).await;

    // A second live `tags` row on R: an assert inserts without folding.
    let beta = serde_json::json!(["beta"]);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::PropertyAssert {
            resource: r,
            key: "tags",
            value: &beta,
            weight: 1.0,
            emitter,
        },
    )
    .await
    .expect("a second tags row asserts");
    tx.commit().await.unwrap();

    // An edge-owned row with the same key text.
    let edge = writes::assert_anchored_edge_with(
        &pool,
        writes::AssertAnchoredEdgeParams {
            source: AnchorRef::resource(r),
            target: AnchorRef::resource(twin),
            kind: temper_substrate::affinity::EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("derived_from"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::Edge { id: edge },
        "tags",
        &serde_json::json!(["alpha"]),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();

    assert_eq!(live_rows(&pool, "kb_resources", r.uuid(), "tags").await, 2);
    let (event, payload) = append_unset(&pool, emitter, r, "tags").await;

    let folded = project_unset(&pool, event, &payload)
        .await
        .expect("the projector runs");

    assert_eq!(folded, 2, "the count is R's live rows of the key");
    assert_eq!(live_rows(&pool, "kb_resources", r.uuid(), "tags").await, 0);
    let stamped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 AND property_key = 'tags' \
            AND is_folded AND last_event_id = $2",
    )
    .bind(r.uuid())
    .bind(event)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stamped, 2, "each folded row carries the unset event");
    assert_eq!(
        live_rows(&pool, "kb_resources", r.uuid(), "descriptor").await,
        1,
        "R's other key is untouched"
    );
    assert_eq!(
        live_rows(&pool, "kb_resources", twin.uuid(), "tags").await,
        1,
        "another resource's row of the same key is untouched"
    );
    assert_eq!(
        live_rows(&pool, "kb_edges", edge.uuid(), "tags").await,
        1,
        "an edge-owned row of the same key text is untouched"
    );
}

/// The write guard runs inside the projector: an erased owner refuses the unset.
///
/// FAILS IF: the function does not exist, or it projects onto an erased resource without
/// raising the guard's refusal.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unset_projector_refuses_an_erased_owner(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(
        &pool,
        owner,
        emitter,
        home,
        "scrub-erased",
        &[("tags".to_string(), serde_json::json!(["alpha"]))],
    )
    .await;
    let erased_event = erase(&pool, r).await;

    // The guard raises before the fold writes anything, so the erasure's own event stands in
    // for the unset event's id.
    let err = project_unset(&pool, erased_event, &unset_payload(r, "tags"))
        .await
        .expect_err("an erased owner refuses the unset");
    let msg = err
        .as_database_error()
        .map(|db| db.message().to_owned())
        .unwrap_or_else(|| err.to_string());
    assert!(
        msg.contains(&format!(
            "resource {} is erased; writes are refused",
            r.uuid()
        )),
        "got {msg}"
    );
}

/// Folding the last `tags` row rebuilds the resource's search vector, so the tag's term no
/// longer matches.
///
/// FAILS IF: the function does not exist, or it folds the row and leaves the stale vector.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_unset_projector_rebuilds_the_search_vector_when_the_last_tags_row_folds(pool: PgPool) {
    let (owner, emitter, home) = setup(&pool).await;
    let r = create(
        &pool,
        owner,
        emitter,
        home,
        "scrub-fts",
        &[("tags".to_string(), serde_json::json!([TAG_TERM]))],
    )
    .await;
    assert!(
        vector_matches(&pool, r, TAG_TERM).await,
        "the tag term is indexed through the create path"
    );
    let before = search_vector(&pool, r).await;
    let (event, payload) = append_unset(&pool, emitter, r, "tags").await;

    let folded = project_unset(&pool, event, &payload)
        .await
        .expect("the projector runs");

    assert_eq!(folded, 1);
    assert_ne!(
        search_vector(&pool, r).await,
        before,
        "the vector is rebuilt"
    );
    assert!(
        !vector_matches(&pool, r, TAG_TERM).await,
        "the folded tag's term no longer matches"
    );
}
