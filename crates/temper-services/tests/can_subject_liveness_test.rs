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
    temper_services::test_support::profile::insert(pool, handle).await
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

/// The capability bits of one grant row. `false` by default so a call site states only the bits
/// it means (`..Capabilities::default()`), mirroring how the SQL table itself defaults them.
#[derive(Default)]
struct Capabilities {
    can_read: bool,
    can_write: bool,
    can_delete: bool,
    can_grant: bool,
}

/// Explicit profile-anchored grant. Capability bits beyond `read` require `read` (the carried
/// coherence CHECK), so callers pass the full bit pattern they mean.
async fn grant(
    pool: &sqlx::PgPool,
    subject_table: &str,
    subject: Uuid,
    principal: Uuid,
    granted_by: Uuid,
    caps: Capabilities,
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
    .bind(caps.can_read)
    .bind(caps.can_write)
    .bind(caps.can_delete)
    .bind(caps.can_grant)
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
        Capabilities {
            can_read: true,
            can_delete: true,
            can_grant: true,
            ..Capabilities::default()
        },
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
        Capabilities {
            can_read: true,
            can_write: true,
            ..Capabilities::default()
        },
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
        Capabilities {
            can_read: true,
            ..Capabilities::default()
        },
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
        Capabilities {
            can_read: true,
            ..Capabilities::default()
        },
    )
    .await;

    assert!(
        can(&pool, holder, "read", "kb_connections", connection).await,
        "a subject kind with no liveness column answers from its grant row — \
         this is the arm an over-broad floor would close"
    );
}

/// Home `resource` in `context`, owned (and originated) by `owner`.
async fn home_resource(pool: &sqlx::PgPool, resource: Uuid, context: Uuid, owner: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(context)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

/// `derived_access_profile(profile, action, 'kb_resources', resource)`, read directly.
async fn derived(pool: &sqlx::PgPool, profile: Uuid, action: &str, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT derived_access_profile($1, $2, 'kb_resources', $3)")
        .bind(profile)
        .bind(action)
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The owner's derived `grant` closes on a tombstone; `delete` does not (migration
/// `20261002000010`). `grant` answers only a live resource, as `read` and `write` already do
/// through their predicates: kb_resource_homes keeps its row when the resource is soft-deleted, so
/// ownership alone would keep it open. `delete` is blob custody and stays with the owner: a soft
/// delete folds no edge, and a floored `delete` would leave a blob related to the tombstone
/// deletable by no one (ruled 2026-10-01).
///
/// FAILS IF `grant` answers a dead resource, or `delete` stops answering its owner there. The
/// bites: drop the `AND EXISTS (... r.is_active)` conjunct from the `grant` arm in
/// `20261002000010` (the grant assertions fail), or add it to the `delete` arm (the custody
/// assertion fails) — each through `derived_access_profile` and through `can()`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_owners_derived_grant_closes_on_a_tombstone_and_delete_custody_stays(
    pool: sqlx::PgPool,
) {
    let owner = insert_profile(&pool, "floor_owner5").await;
    let resource = insert_resource(&pool, "floor-owned-doc").await;
    let context = insert_context(&pool, owner, "floor-owned-ctx").await;
    home_resource(&pool, resource, context, owner).await;

    // Live subject: home ownership derives both, and the seam agrees.
    for action in ["grant", "delete"] {
        assert!(
            derived(&pool, owner, action, resource).await,
            "the owner derives {action} on a live resource"
        );
        assert!(
            can(&pool, owner, action, "kb_resources", resource).await,
            "can({action}) admits the owner of a live resource"
        );
    }

    set_resource_active(&pool, resource, false).await;

    // Tombstone: grant administration closes; blob custody stays with the owner.
    assert!(
        !derived(&pool, owner, "grant", resource).await,
        "the owner derives no grant on a tombstone"
    );
    assert!(
        !can(&pool, owner, "grant", "kb_resources", resource).await,
        "can(grant) refuses the owner of a tombstone"
    );
    assert!(
        derived(&pool, owner, "delete", resource).await,
        "the owner keeps delete (blob custody) on a tombstone"
    );
    assert!(
        can(&pool, owner, "delete", "kb_resources", resource).await,
        "can(delete) still admits the owner of a tombstone"
    );
}
