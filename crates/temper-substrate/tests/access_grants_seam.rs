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

/// Home the resource in a fresh context owned by `owner`, and return the context id.
async fn home_resource(pool: &sqlx::PgPool, resource: Uuid, owner: Uuid, slug: &str) -> Uuid {
    let context: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
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
    context
}

/// The `delete` arm of `derived_access_profile` (migration `20260718000030`).
///
/// It exists so that attenuating grant-administration does not deadlock `can_delete`: with no
/// derivable holder anywhere and zero grant rows carrying it, no delegated administrator could ever
/// confer delete, so nobody would ever come to hold it. This pins BOTH halves of the decision — that
/// the resource-home owner derives it, and that the scoping to `kb_resources` is deliberate rather
/// than an oversight a later reader should "complete".
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn resource_home_owner_derives_delete_and_nobody_else_does(pool: sqlx::PgPool) {
    let owner = insert_profile(&pool, "delete-owner").await;
    let stranger = insert_profile(&pool, "delete-stranger").await;
    let resource = insert_resource(&pool, "owned", "test://owned").await;
    let context = home_resource(&pool, resource, owner, "delete-ctx").await;

    assert!(
        can(
            &pool,
            "kb_profiles",
            owner,
            "delete",
            "kb_resources",
            resource
        )
        .await,
        "the owner of the resource's home must derive delete on it"
    );
    assert!(
        !can(
            &pool,
            "kb_profiles",
            stranger,
            "delete",
            "kb_resources",
            resource
        )
        .await,
        "a profile with no home ownership must not derive delete"
    );

    // Non-vacuity: the stranger is not simply invisible to every arm — it is the DELETE arm that
    // refuses. Were the fixture broken (no home, wrong ids), the owner's `grant` probe would fail
    // too and the assertion above would pass for the wrong reason.
    assert!(
        can(
            &pool,
            "kb_profiles",
            owner,
            "grant",
            "kb_resources",
            resource
        )
        .await,
        "fixture check: the same ownership must already satisfy the pre-existing grant arm"
    );

    // The scoping is deliberate (see the migration's COMMENT ON FUNCTION): contexts and cogmaps get
    // NO derived delete arm, so even the context's OWN owner does not derive delete on it. Adding
    // one is a design question about those subject types, not a consequence of attenuation.
    assert!(
        !can(
            &pool,
            "kb_profiles",
            owner,
            "delete",
            "kb_contexts",
            context
        )
        .await,
        "kb_contexts must stay fail-closed on delete even for its owner"
    );
}
