#![cfg(feature = "test-db")]

//! D1 — `originator_profile_id` confers NO access; `owner_profile_id` is the access-bearing
//! profile key. Provenance ≠ access. See the context-transfer-safety spec.

use uuid::Uuid;

async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_resource(pool: &sqlx::PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id")
        .bind(title)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_context(pool: &sqlx::PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn home_resource(
    pool: &sqlx::PgPool,
    resource: Uuid,
    context: Uuid,
    originator: Uuid,
    owner: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $4)",
    )
    .bind(resource)
    .bind(context)
    .bind(originator)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

async fn can_read(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) WHERE resource_id = $2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn can_modify(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Behavior-preserving: a creator (owner == originator) still reads + modifies their resource.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn owner_that_is_also_originator_keeps_access(pool: sqlx::PgPool) {
    let alice = insert_profile(&pool, "alice").await;
    let ctx = insert_context(&pool, alice, "alice-ctx").await;
    let res = insert_resource(&pool, "doc").await;
    home_resource(&pool, res, ctx, alice, alice).await; // originator == owner == alice

    assert!(
        can_read(&pool, alice, res).await,
        "creator reads their own resource"
    );
    assert!(
        can_modify(&pool, alice, res).await,
        "creator modifies their own resource"
    );
}

/// The D1 change: when owner and originator DIVERGE, only the OWNER has access — the
/// originator (former creator, now handed off) is cut off on both axes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn originator_without_ownership_has_no_access(pool: sqlx::PgPool) {
    let alice = insert_profile(&pool, "alice").await; // originator (former owner)
    let bob = insert_profile(&pool, "bob").await; // current owner (post-handoff)
    let ctx = insert_context(&pool, bob, "bob-ctx").await;
    let res = insert_resource(&pool, "doc").await;
    home_resource(&pool, res, ctx, alice, bob).await; // originator=alice, owner=bob

    assert!(can_read(&pool, bob, res).await, "owner reads");
    assert!(can_modify(&pool, bob, res).await, "owner modifies");
    assert!(
        !can_read(&pool, alice, res).await,
        "bare originator does NOT read"
    );
    assert!(
        !can_modify(&pool, alice, res).await,
        "bare originator does NOT modify"
    );
}
