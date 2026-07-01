#![cfg(feature = "artifact-tests")]
//! Deliverable 2 of the generalized access-capability arc (design doc
//! `docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md` §3.3/§3.5/§4 step 1):
//! the `kb_access_grants` table + the `can()` seam land **alongside** the existing access functions with
//! **no behavior change**. These tests pin the new affordance and the leak-safety invariant (Q-B):
//!
//!   1. the coherence CHECK (`write|delete|grant ⇒ read`) fires on the new dual-polymorphic table;
//!   2. `can('kb_profiles', …, 'read', 'kb_resources', …)` is at **parity** with `resources_visible_to`
//!      for a resource with no explicit grant, and an explicit `kb_access_grants` row **flips both true**
//!      (via `profile_explicit_grant` for `can()`, and — since the D5 store swap — via
//!      `resources_visible_to` reading `kb_access_grants` directly);
//!   3. the **Cogmap** arm of `can()` takes **no** explicit grants (Q-B): a profile-axis grant never leaks
//!      into the producer intersection.
//!
//! Minimal-anchor pattern (mirrors `access_scenario::s8_capability_check_rejects_write_without_read`):
//! each `#[sqlx::test]` gets a fresh migrated `public`-schema DB; runtime `sqlx::query` (no macros, no
//! per-crate `.sqlx` cache needed); no ONNX (no charter embeds here).

use sqlx::Row;
use uuid::Uuid;

/// Insert a minimal profile and return its id. `system_access='none'` avoids the root-join trigger so
/// the profile reaches nothing by default (a clean baseline for the parity assertion).
async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ($1, $1, 'none') RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_resource(pool: &sqlx::PgPool, title: &str, uri: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(uri)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn can(
    pool: &sqlx::PgPool,
    principal_table: &str,
    principal: Uuid,
    action: &str,
    subject_table: &str,
    subject: Uuid,
) -> bool {
    sqlx::query_scalar("SELECT can($1, $2, $3, $4, $5)")
        .bind(principal_table)
        .bind(principal)
        .bind(action)
        .bind(subject_table)
        .bind(subject)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn visible_to(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn access_grants_coherence_check_rejects_write_without_read(pool: sqlx::PgPool) {
    let granter = insert_profile(&pool, "ag_granter").await;
    // kb_access_grants.subject_id is polymorphic (no FK), so a synthetic cogmap id suffices —
    // this test exercises the coherence CHECK, not any cogmap row.
    let cogmap: Uuid = sqlx::query_scalar("SELECT gen_random_uuid()")
        .fetch_one(&pool)
        .await
        .unwrap();

    // A cogmap-subject grant with can_write=true, can_read=false must be rejected — the carried
    // coherence CHECK (`(can_write OR can_delete OR can_grant) <= can_read`) holds on the new table.
    let res = sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, false, true, $2)",
    )
    .bind(cogmap)
    .bind(granter)
    .execute(&pool)
    .await;

    let err = res.expect_err("write-without-read grant must be rejected on kb_access_grants");
    let is_check_violation = matches!(
        &err,
        sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")
    );
    assert!(
        is_check_violation,
        "expected check_violation (23514), got {err:?}"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn can_seam_parity_then_explicit_grant_flips_read(pool: sqlx::PgPool) {
    let alice = insert_profile(&pool, "ag_alice").await;
    let r = insert_resource(&pool, "ag-doc", "temper://ag-doc").await;

    // Baseline: alice reaches nothing; the resource has no home/grant she can see.
    // PARITY — can(read) tracks resources_visible_to exactly when there is no explicit grant.
    assert!(
        !visible_to(&pool, alice, r).await,
        "precondition: resource not visible"
    );
    assert!(
        !can(&pool, "kb_profiles", alice, "read", "kb_resources", r).await,
        "can(read) must match resources_visible_to (both false) before any grant"
    );

    // Land an explicit profile-anchored read grant in the NEW table.
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, $2)",
    )
    .bind(r)
    .bind(alice)
    .execute(&pool)
    .await
    .unwrap();

    // can() now sees it (via profile_explicit_grant reading kb_access_grants).
    assert!(
        can(&pool, "kb_profiles", alice, "read", "kb_resources", r).await,
        "explicit kb_access_grants row must flip can(read) true"
    );
    // D5 STORE SWAP: resources_visible_to now reads kb_access_grants (subject_table='kb_resources')
    // directly — a profile-anchored read grant confers visibility, restoring parity with can(read) at
    // the true level (both true after the grant).
    assert!(
        visible_to(&pool, alice, r).await,
        "resources_visible_to must now honor the kb_access_grants resource read grant (D5)"
    );
    // And write was not granted — can(write) stays false (the grant set only can_read).
    assert!(
        !can(&pool, "kb_profiles", alice, "write", "kb_resources", r).await,
        "a read-only grant must not confer write"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn can_seam_cogmap_arm_ignores_explicit_grants(pool: sqlx::PgPool) {
    // Q-B: explicit grants are Profile-axis only; they never enter the Cogmap producer intersection.
    let alice = insert_profile(&pool, "ag_alice2").await;
    let r = insert_resource(&pool, "ag-doc2", "temper://ag-doc2").await;
    // A team-less cogmap principal: resources_accessible_to_cogmap(.) is empty (it joins
    // kb_team_cogmaps, which has no rows for this id), so its producer reach is empty regardless
    // of any grant. The principal id need not reference a real kb_cogmaps row.
    let cogmap: Uuid = sqlx::query_scalar("SELECT gen_random_uuid()")
        .fetch_one(&pool)
        .await
        .unwrap();

    // A profile-anchored read grant on r exists…
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, $2)",
    )
    .bind(r)
    .bind(alice)
    .execute(&pool)
    .await
    .unwrap();

    // …but the Cogmap principal's reach is the producer intersection (empty for a team-less map),
    // which never consults kb_access_grants. The grant must NOT leak in.
    assert!(
        !can(&pool, "kb_cogmaps", cogmap, "read", "kb_resources", r).await,
        "Q-B: a profile-axis grant must never enter the Cogmap producer reach"
    );
    // Non-resource subject / non-read action on the Cogmap axis ⇒ false (agents don't administer/author qua principal).
    assert!(
        !can(&pool, "kb_cogmaps", cogmap, "grant", "kb_resources", r).await,
        "Cogmap axis admits only read of resource subjects"
    );

    // sanity: the helper row count is what we inserted (guards against a silently-dropped insert).
    let n: i64 = sqlx::query("SELECT count(*) FROM kb_access_grants")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}
