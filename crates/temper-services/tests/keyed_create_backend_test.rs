//! Integration test — client-supplied resource id as the create idempotency key (spike rung 3,
//! issue #581). A create carrying a `resource_id` mints the resource *under that id*; a replay of
//! the same keyed create converges on the already-committed resource instead of minting a twin
//! (the safe recovery for a lost-acknowledgment retry). A replay of a **foreign** id is refused
//! leak-safely as `NotFound` — never 403/409, so a stable id can't become an existence oracle.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write path's
/// `resolve_emitter` + visibility gate require). Each test-target crate keeps its own copy so it
/// has no cross-target harness dependency — mirrors `segmented_backend_test.rs`.
async fn seed_profile_with_context(
    pool: &PgPool,
    email: &str,
    slug: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,$3,$3)",
    )
    .bind(context_id)
    .bind(profile_id)
    .bind(slug)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// Build a `CreateResource` for the owned `temper` context, carrying `resource_id`.
fn keyed_create(
    context: uuid::Uuid,
    slug: &str,
    resource_id: Option<uuid::Uuid>,
) -> CreateResource {
    CreateResource {
        resource_id,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: slug.to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal: None,
        origin_uri: Some(format!("test://{slug}")),
        chunks_packed: None,
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// Count live resources homed in a specific context. `kb_resources` carries no owner column
/// (ownership is derived via the home), so a per-context count is what isolates a test's own writes
/// from the L0 kernel resources a migrated DB already seeds. Each test uses a fresh context.
async fn homed_count(pool: &PgPool, context: uuid::Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = $1 AND r.is_active",
    )
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("count homed resources")
}

/// A create carrying a fresh client-minted id mints the resource *under that exact id*.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn keyed_create_mints_under_the_supplied_id(pool: PgPool) {
    let (profile, context) =
        seed_profile_with_context(&pool, "keyed-mint@example.com", "temper").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let id = uuid::Uuid::now_v7();

    let row = backend
        .create_resource(keyed_create(context, "keyed-doc", Some(id)))
        .await
        .expect("keyed create")
        .value;

    assert_eq!(
        row.id,
        ResourceId::from(id),
        "the resource must be born under the supplied id"
    );
    assert_eq!(homed_count(&pool, context).await, 1);
}

/// Replaying the SAME keyed create converges on the already-committed resource — no twin, no
/// error. This is the lost-acknowledgment recovery: the client retries with the same id and gets
/// the existing resource back.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn replaying_a_keyed_create_returns_the_existing_resource(pool: PgPool) {
    let (profile, context) =
        seed_profile_with_context(&pool, "keyed-replay@example.com", "temper").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let id = uuid::Uuid::now_v7();

    let first = backend
        .create_resource(keyed_create(context, "keyed-doc", Some(id)))
        .await
        .expect("first keyed create")
        .value;
    let second = backend
        .create_resource(keyed_create(context, "keyed-doc", Some(id)))
        .await
        .expect("replayed keyed create must succeed, not conflict")
        .value;

    assert_eq!(first.id, ResourceId::from(id));
    assert_eq!(second.id, first.id, "replay must return the same resource");
    assert_eq!(
        homed_count(&pool, context).await,
        1,
        "a replay must not mint a duplicate"
    );
}

/// A create replaying a FOREIGN id (one that belongs to another principal) is refused as
/// `NotFound` — never `Forbidden`/`Conflict` — so a stable id discloses nothing a GET would not,
/// and the other principal's resource is untouched.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn keyed_create_on_a_foreign_id_is_notfound_and_leaves_the_owner_untouched(pool: PgPool) {
    // Principal A owns id X.
    let (profile_a, context_a) =
        seed_profile_with_context(&pool, "owner-a@example.com", "temper").await;
    let backend_a = DbBackend::new(pool.clone(), ProfileId::from(profile_a));
    let id = uuid::Uuid::now_v7();
    let a_row = backend_a
        .create_resource(keyed_create(context_a, "a-doc", Some(id)))
        .await
        .expect("A creates under id X")
        .value;
    assert_eq!(a_row.id, ResourceId::from(id));

    // Principal B (a different owner + context) replays a create under A's id.
    let (profile_b, context_b) =
        seed_profile_with_context(&pool, "intruder-b@example.com", "temper-b").await;
    let backend_b = DbBackend::new(pool.clone(), ProfileId::from(profile_b));
    let err = backend_b
        .create_resource(keyed_create(context_b, "b-doc", Some(id)))
        .await
        .expect_err("B must not adopt A's id");

    match err {
        TemperError::NotFound(_) => {}
        other => panic!("foreign-id keyed create must be NotFound (leak-safe), got {other:?}"),
    }
    // A still owns exactly her one resource, unchanged (owner lives on the home, not kb_resources).
    assert_eq!(homed_count(&pool, context_a).await, 1);
    let owner: uuid::Uuid = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT owner_profile_id FROM kb_resource_homes WHERE resource_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("A's resource still present");
    assert_eq!(owner, profile_a, "A's resource must be untouched");
}

/// A create with NO supplied id keeps today's behaviour: a fresh id is minted, and two such
/// creates are distinct resources (the id is the identity, not the body).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn unkeyed_create_mints_fresh_each_time(pool: PgPool) {
    let (profile, context) =
        seed_profile_with_context(&pool, "unkeyed@example.com", "temper").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let a = backend
        .create_resource(keyed_create(context, "doc", None))
        .await
        .expect("first create")
        .value;
    let b = backend
        .create_resource(keyed_create(context, "doc", None))
        .await
        .expect("second create")
        .value;

    assert_ne!(a.id, b.id, "unkeyed creates are distinct resources");
    assert_eq!(homed_count(&pool, context).await, 2);
}
