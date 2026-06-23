#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! Dark-launch coverage for the substrate (`temper_next.*`) context path.
//!
//! Exercises the `#[cfg(feature = "next-backend")]` `*_next` service variants
//! against the grafted substrate schema. Fixtures seed `temper_next.*` directly
//! with raw queries (no macros → no `.sqlx` cache entries for the test target).

mod common;

use sqlx::PgPool;
use temper_api::services::context_service;
use temper_core::types::ids::ProfileId;
use uuid::Uuid;

/// Seed a bare substrate profile and return its id.
///
/// The `kb_profiles` insert fires the `sync_personal_team` trigger, whose body
/// references `kb_teams` unqualified — so the seed runs inside a transaction
/// with `search_path = temper_next` to keep that resolution off the legacy
/// `public` twin (whose `kb_teams.id` has no default).
async fn seed_next_profile(pool: &PgPool, label: &str) -> ProfileId {
    let id = Uuid::now_v7();
    let handle = format!("{label}-{}", &id.simple().to_string()[..8]);
    let mut tx = pool.begin().await.expect("begin seed tx");
    sqlx::query("SET LOCAL search_path = temper_next, public")
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
    tx.commit().await.expect("commit seed tx");
    ProfileId(id)
}

/// Home a fresh substrate resource into a context, returning its id.
async fn home_resource_in_context(pool: &PgPool, owner: ProfileId, context_id: Uuid) -> Uuid {
    let resource_id = Uuid::now_v7();
    sqlx::query("INSERT INTO temper_next.kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(resource_id)
        .bind("Homed Doc")
        .bind(format!("temper://test/{resource_id}"))
        .execute(pool)
        .await
        .expect("seed substrate resource");

    sqlx::query(
        "INSERT INTO temper_next.kb_resource_homes
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource_id)
    .bind(context_id)
    .bind(*owner)
    .execute(pool)
    .await
    .expect("home substrate resource");

    resource_id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_next_generates_slug_and_lists_for_owner(pool: PgPool) {
    let owner = seed_next_profile(&pool, "owner").await;

    let created = context_service::create_next(&pool, owner, "My Substrate Context")
        .await
        .expect("create_next");
    assert_eq!(created.name, "My Substrate Context");
    assert_eq!(created.kb_owner_table, "kb_profiles");
    assert_eq!(created.kb_owner_id, *owner);

    // A slug was generated from the name on the stored row.
    let slug: String = sqlx::query_scalar("SELECT slug FROM temper_next.kb_contexts WHERE id = $1")
        .bind(*created.id)
        .fetch_one(&pool)
        .await
        .expect("read generated slug");
    assert_eq!(slug, "my-substrate-context");

    // Owner sees it with a zero resource count.
    let listed = context_service::list_visible_next(&pool, owner)
        .await
        .expect("list_visible_next");
    let row = listed
        .iter()
        .find(|c| c.id == created.id)
        .expect("created context visible to owner");
    assert_eq!(row.resource_count, 0);

    // Homing a resource bumps the count to 1.
    home_resource_in_context(&pool, owner, *created.id).await;
    let listed = context_service::list_visible_next(&pool, owner)
        .await
        .expect("list_visible_next after home");
    let row = listed
        .iter()
        .find(|c| c.id == created.id)
        .expect("context still visible");
    assert_eq!(row.resource_count, 1);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_visible_next_hides_unshared_context_from_non_owner(pool: PgPool) {
    let owner = seed_next_profile(&pool, "owner").await;
    let stranger = seed_next_profile(&pool, "stranger").await;

    let created = context_service::create_next(&pool, owner, "Private Context")
        .await
        .expect("create_next");

    // The stranger has no kb_team_contexts share, so the context is invisible.
    let listed = context_service::list_visible_next(&pool, stranger)
        .await
        .expect("list_visible_next for stranger");
    assert!(
        !listed.iter().any(|c| c.id == created.id),
        "unshared context must not be visible to a non-owner"
    );

    // get_visible_next and resolve_by_name_next are also gated for the stranger.
    assert!(
        context_service::get_visible_next(&pool, stranger, created.id)
            .await
            .is_err(),
        "get_visible_next must deny a non-owner"
    );
    assert!(
        context_service::resolve_by_name_next(&pool, stranger, "Private Context")
            .await
            .is_err(),
        "resolve_by_name_next must deny a non-owner"
    );

    // The owner can still resolve it by name and id.
    let by_name = context_service::resolve_by_name_next(&pool, owner, "Private Context")
        .await
        .expect("owner resolve_by_name_next");
    assert_eq!(by_name.id, created.id);
    let by_id = context_service::get_visible_next(&pool, owner, created.id)
        .await
        .expect("owner get_visible_next");
    assert_eq!(by_id.id, created.id);
}
