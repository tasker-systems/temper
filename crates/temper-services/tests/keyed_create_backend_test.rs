//! Integration test — owner-scoped create idempotency key (issue #581, spike rung 3).
//!
//! A create carrying an `idempotency_key` claims the `(owner, key)` slot atomically with the create;
//! a replay of the same key by the same owner converges on the already-committed resource instead of
//! minting a twin. The key is **owner-scoped**, which is what makes it safe: the *same key value*
//! supplied by a *different* owner does not collide and is not refused — it simply mints a fresh
//! resource in that owner's namespace. So a key can never be an existence oracle (a foreign key is
//! absent from your namespace, exactly as a read cannot tell nonexistent from hidden). The server
//! always mints the resource id; the client never supplies one.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile and a profile-owned context (the minimum the write path's
/// `resolve_emitter` and visibility gate require). Each test-target crate keeps its own copy,
/// mirroring `segmented_backend_test.rs`.
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

/// Build a `CreateResource` for the owned context, carrying an optional idempotency key.
fn create_cmd(
    context: uuid::Uuid,
    slug: &str,
    idempotency_key: Option<uuid::Uuid>,
) -> CreateResource {
    CreateResource {
        idempotency_key,
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

/// Count live resources homed in a context — isolates a test's own writes from the L0 kernel
/// resources a migrated DB seeds. Each test uses a fresh context.
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

/// A keyed create mints a resource under a **server-generated** id (never the key), and records the
/// `(owner, key) → resource` mapping.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn keyed_create_mints_a_resource_and_records_the_mapping(pool: PgPool) {
    let (profile, context) =
        seed_profile_with_context(&pool, "keyed-mint@example.com", "temper").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let key = uuid::Uuid::now_v7();

    let row = backend
        .create_resource(create_cmd(context, "keyed-doc", Some(key)))
        .await
        .expect("keyed create")
        .value;

    assert_ne!(
        uuid::Uuid::from(row.id),
        key,
        "the resource id is server-minted, never the client's key"
    );
    assert_eq!(homed_count(&pool, context).await, 1);
    let mapped: uuid::Uuid = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT resource_id FROM kb_idempotency_keys WHERE owner_profile_id = $1 AND idempotency_key = $2",
    )
    .bind(profile)
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("mapping row recorded");
    assert_eq!(
        mapped,
        uuid::Uuid::from(row.id),
        "mapping points at the minted resource"
    );
}

/// Replaying the SAME key by the SAME owner converges on the already-committed resource — no twin,
/// no error. The lost-acknowledgment recovery.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn replaying_a_key_returns_the_same_resource(pool: PgPool) {
    let (profile, context) =
        seed_profile_with_context(&pool, "keyed-replay@example.com", "temper").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let key = uuid::Uuid::now_v7();

    let first = backend
        .create_resource(create_cmd(context, "keyed-doc", Some(key)))
        .await
        .expect("first keyed create")
        .value;
    let second = backend
        .create_resource(create_cmd(context, "keyed-doc-attempt-2", Some(key)))
        .await
        .expect("replay must succeed, not conflict")
        .value;

    assert_eq!(second.id, first.id, "replay returns the original resource");
    assert_eq!(
        homed_count(&pool, context).await,
        1,
        "a replay mints no duplicate"
    );
}

/// The key is **owner-scoped**: the SAME key value supplied by a DIFFERENT owner does not collide,
/// is not refused (no 404), and mints a fresh resource in that owner's namespace. This is the
/// no-oracle property — a foreign key is simply absent from your namespace.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_same_key_from_a_different_owner_mints_fresh_no_oracle(pool: PgPool) {
    let key = uuid::Uuid::now_v7();

    let (profile_a, context_a) =
        seed_profile_with_context(&pool, "owner-a@example.com", "temper").await;
    let backend_a = DbBackend::new(pool.clone(), ProfileId::from(profile_a));
    let a = backend_a
        .create_resource(create_cmd(context_a, "a-doc", Some(key)))
        .await
        .expect("A creates under key K")
        .value;

    let (profile_b, context_b) =
        seed_profile_with_context(&pool, "owner-b@example.com", "temper-b").await;
    let backend_b = DbBackend::new(pool.clone(), ProfileId::from(profile_b));
    let b = backend_b
        .create_resource(create_cmd(context_b, "b-doc", Some(key)))
        .await
        .expect("B's create under the same key K must succeed (owner-scoped), not 404 or collide")
        .value;

    assert_ne!(
        b.id, a.id,
        "B mints its own resource; it does not adopt or observe A's"
    );
    assert_eq!(homed_count(&pool, context_a).await, 1, "A untouched");
    assert_eq!(
        homed_count(&pool, context_b).await,
        1,
        "B got its own resource"
    );
    // Two independent mappings, one per owner.
    let rows: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_idempotency_keys WHERE idempotency_key = $1",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("count key mappings");
    assert_eq!(
        rows, 2,
        "one (owner,key) mapping per owner — namespaces are disjoint"
    );
}

/// A create with no key keeps today's behaviour: a fresh id each time, distinct resources.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn unkeyed_create_mints_fresh_each_time(pool: PgPool) {
    let (profile, context) =
        seed_profile_with_context(&pool, "unkeyed@example.com", "temper").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let a = backend
        .create_resource(create_cmd(context, "doc", None))
        .await
        .expect("first create")
        .value;
    let b = backend
        .create_resource(create_cmd(context, "doc", None))
        .await
        .expect("second create")
        .value;

    assert_ne!(a.id, b.id, "unkeyed creates are distinct resources");
    assert_eq!(homed_count(&pool, context).await, 2);
}
