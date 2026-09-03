#![cfg(feature = "test-db")]

//! `can()`'s profile arm has two branches that must answer a subject's liveness identically: the
//! derived floor delegates to the concrete predicates, every one of which semi-joins
//! `kb_resources.is_active` / `kb_contexts.is_active`, while `profile_explicit_grant` is
//! subject-polymorphic and reads only `kb_access_grants`. The floor therefore lives in `can()`
//! itself, on the explicit branch — the same delegation shape `context_authorable_by_profile`
//! applies to its own `profile_explicit_grant` arm. Subject kinds with no liveness column
//! (`kb_cogmaps`, `kb_connections`) are deliberately unfloored: a grant row there stays
//! answerable (20260714000020), and a future grantable kind joins the floor's CASE as part of
//! its own design.
//!
//! These witnesses hold the seam to the concrete gates it unifies: `can()` and the derived gate
//! must give the same answer for a tombstoned subject, and the same answer for a live one.

use sqlx::Row;
use uuid::Uuid;

async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_resource(pool: &sqlx::PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id")
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

/// Explicit profile-anchored grant. Capability bits beyond `read` require `read` (the carried
/// coherence CHECK), so callers pass the full bit pattern they mean.
async fn grant(
    pool: &sqlx::PgPool,
    subject_table: &str,
    subject: Uuid,
    principal: Uuid,
    granted_by: Uuid,
    can_read: bool,
    can_write: bool,
    can_delete: bool,
    can_grant: bool,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
         VALUES ($1, $2, 'kb_profiles', $3, $4, $5, $6, $7, $8)",
    )
    .bind(subject_table)
    .bind(subject)
    .bind(principal)
    .bind(can_read)
    .bind(can_write)
    .bind(can_delete)
    .bind(can_grant)
    .bind(granted_by)
    .execute(pool)
    .await
    .unwrap();
}

async fn set_resource_active(pool: &sqlx::PgPool, resource: Uuid, active: bool) {
    sqlx::query("UPDATE kb_resources SET is_active = $2 WHERE id = $1")
        .bind(resource)
        .bind(active)
        .execute(pool)
        .await
        .unwrap();
}

async fn set_context_active(pool: &sqlx::PgPool, context: Uuid, active: bool) {
    sqlx::query("UPDATE kb_contexts SET is_active = $2 WHERE id = $1")
        .bind(context)
        .bind(active)
        .execute(pool)
        .await
        .unwrap();
}

async fn can(
    pool: &sqlx::PgPool,
    principal: Uuid,
    action: &str,
    subject_table: &str,
    subject: Uuid,
) -> bool {
    sqlx::query_scalar("SELECT can('kb_profiles', $1, $2, $3, $4)")
        .bind(principal)
        .bind(action)
        .bind(subject_table)
        .bind(subject)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The derived read gate's answer for a resource — the gate `can()` must agree with.
async fn resource_visible(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The derived write gate's answer for a context — the gate `can()` must agree with.
async fn context_authorable(pool: &sqlx::PgPool, profile: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_tombstoned_resource_closes_cans_explicit_grant_arm(pool: sqlx::PgPool) {
    let grantor = insert_profile(&pool, "floor_grantor").await;
    let holder = insert_profile(&pool, "floor_holder").await;
    let resource = insert_resource(&pool, "floor-doc").await;

    grant(
        &pool,
        "kb_resources",
        resource,
        holder,
        grantor,
        true,  // can_read
        false, // can_write
        true,  // can_delete
        true,  // can_grant
    )
    .await;

    // Live subject: the explicit arm admits, and the derived gate agrees.
    assert!(
        resource_visible(&pool, holder, resource).await,
        "fixture: an explicit read grant makes the live resource visible"
    );
    assert!(
        can(&pool, holder, "read", "kb_resources", resource).await,
        "a live subject stays admitted on the explicit arm"
    );
    assert!(
        can(&pool, holder, "delete", "kb_resources", resource).await,
        "a live subject stays admitted for a granted delete"
    );
    assert!(
        can(&pool, holder, "grant", "kb_resources", resource).await,
        "a live subject stays admitted for a granted grant capability"
    );

    set_resource_active(&pool, resource, false).await;

    // The derived gate closes (its semi-join drops the row)…
    assert!(
        !resource_visible(&pool, holder, resource).await,
        "the derived read gate must close on a tombstoned resource"
    );
    // …and the seam must answer the same: the explicit arm no longer admits.
    assert!(
        !can(&pool, holder, "read", "kb_resources", resource).await,
        "can(read) must agree with the derived gate on a tombstoned resource"
    );
    assert!(
        !can(&pool, holder, "delete", "kb_resources", resource).await,
        "can(delete) must agree with the tombstone floor"
    );
    assert!(
        !can(&pool, holder, "grant", "kb_resources", resource).await,
        "can(grant) must agree with the tombstone floor"
    );
    // Ungranted actions stay refused for the right reason: the derived write floor
    // (can_modify_resource) closes independently of the explicit arm.
    assert!(
        !can(&pool, holder, "write", "kb_resources", resource).await,
        "can(write) stays closed on a tombstoned resource"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_retired_context_leaves_can_agreeing_with_the_write_gate(pool: sqlx::PgPool) {
    let grantor = insert_profile(&pool, "floor_grantor2").await;
    let holder = insert_profile(&pool, "floor_holder2").await;
    let context = insert_context(&pool, grantor, "floor-ctx").await;

    grant(
        &pool,
        "kb_contexts",
        context,
        holder,
        grantor,
        true,  // can_read
        true,  // can_write
        false, // can_delete
        false, // can_grant
    )
    .await;

    // Live context: the write gate admits the holder through the explicit grant, and the seam
    // agrees on both axes.
    assert!(
        context_authorable(&pool, holder, context).await,
        "fixture: the explicit write grant admits the live context"
    );
    assert!(
        can(&pool, holder, "write", "kb_contexts", context).await,
        "a live context stays admitted on the explicit arm"
    );
    assert!(
        can(&pool, holder, "read", "kb_contexts", context).await,
        "a live context stays admitted for a granted read"
    );

    set_context_active(&pool, context, false).await;

    // The write gate floors the grant arm at its own delegation, so it closes…
    assert!(
        !context_authorable(&pool, holder, context).await,
        "the write gate must close on a retired context"
    );
    // …and the seam must give the same answer rather than disagreeing with it.
    assert!(
        !can(&pool, holder, "write", "kb_contexts", context).await,
        "can(write) must agree with the write gate on a retired context"
    );
    assert!(
        !can(&pool, holder, "read", "kb_contexts", context).await,
        "can(read) must agree with the derived read gate on a retired context"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_grant_row_on_a_subject_that_was_never_created_answers_false(pool: sqlx::PgPool) {
    let grantor = insert_profile(&pool, "floor_grantor3").await;
    let holder = insert_profile(&pool, "floor_holder3").await;
    // kb_access_grants.subject_id carries no FK (the integrity is the CHECK + the granting path),
    // so a row can name an id no subject row ever backs.
    let dangling: Uuid = sqlx::query_scalar("SELECT gen_random_uuid()")
        .fetch_one(&pool)
        .await
        .unwrap();

    grant(
        &pool,
        "kb_resources",
        dangling,
        holder,
        grantor,
        true,  // can_read
        false, // can_write
        false, // can_delete
        false, // can_grant
    )
    .await;

    // Non-vacuity: the row landed; the refusal below is the floor's answer, not a missing fixture.
    let n: i64 = sqlx::query("SELECT count(*) FROM kb_access_grants")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "fixture: exactly the dangling grant row exists");

    assert!(
        !can(&pool, holder, "read", "kb_resources", dangling).await,
        "a grant row with no subject row behind it must not answer true"
    );
}

/// The floor's CASE admits subject kinds with no liveness column (`ELSE true`) — and this is
/// load-bearing, not cosmetic: for `kb_resources`/`kb_contexts` the derived branch independently
/// answers every live subject, so only a kind with NO derived arm can witness that the floor
/// admits. `kb_connections` is such a kind (derived_access_profile answers false there; the
/// grant row is the whole answer, per 20260714000020). The synthetic id follows the seam tests'
/// minimal-anchor pattern: the probe exercises the floor's dispatch, not any connection row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_subject_kind_without_a_liveness_column_stays_answerable(pool: sqlx::PgPool) {
    let grantor = insert_profile(&pool, "floor_grantor4").await;
    let holder = insert_profile(&pool, "floor_holder4").await;
    let connection: Uuid = sqlx::query_scalar("SELECT gen_random_uuid()")
        .fetch_one(&pool)
        .await
        .unwrap();

    grant(
        &pool,
        "kb_connections",
        connection,
        holder,
        grantor,
        true,  // can_read
        false, // can_write
        false, // can_delete
        false, // can_grant
    )
    .await;

    assert!(
        can(&pool, holder, "read", "kb_connections", connection).await,
        "a subject kind with no liveness column answers from its grant row — \
         this is the arm an over-broad floor would close"
    );
}
