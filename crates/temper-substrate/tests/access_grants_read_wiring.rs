#![cfg(feature = "artifact-tests")]
//! Deliverable 3a of the generalized access-capability arc (design doc §4 step 2, read half): an
//! explicit `kb_access_grants` read row on a **cogmap** or **context** subject confers read WITHOUT
//! team membership — the headline "grant profile X read on cogmap Y" case that is inexpressible in the
//! resources-only `kb_resource_access`. The wiring is purely ADDITIVE (a parallel grant branch beside
//! each membership branch), so a profile with NO grant reads exactly as before.
//!
//! Coverage:
//!   • cogmap read-grant ⇒ `cogmap_readable_by_profile` (shape), the cogmap's homed resource in
//!     `resources_visible_to`, and the map in `cogmap_visible_maps` (wayfind admission);
//!   • context read-grant ⇒ `context_visible_to` and the context's homed resource in `resources_visible_to`;
//!   • a non-granted profile sees none of it (no behavior change).
//!
//! Synthetic polymorphic anchors: `kb_resource_homes` and `kb_access_grants` carry NO FK on their
//! anchor/subject columns, so a generated uuid stands in for the cogmap/context without a real row —
//! the membership paths (which we are NOT exercising) are the only things that would need real rows.

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

async fn fresh_uuid(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT gen_random_uuid()")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A resource homed in `anchor_table`/`anchor_id`, owned+originated by `owner` (so it is NOT visible
/// to anyone else by ownership). Returns the resource id.
async fn insert_homed_resource(
    pool: &sqlx::PgPool,
    title: &str,
    uri: &str,
    anchor_table: &str,
    anchor_id: Uuid,
    owner: Uuid,
) -> Uuid {
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,$2) RETURNING id",
    )
    .bind(title)
    .bind(uri)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(rid)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    rid
}

async fn grant_read(
    pool: &sqlx::PgPool,
    subject_table: &str,
    subject: Uuid,
    profile: Uuid,
    granter: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ($1, $2, 'kb_profiles', $3, true, $4)",
    )
    .bind(subject_table)
    .bind(subject)
    .bind(profile)
    .bind(granter)
    .execute(pool)
    .await
    .unwrap();
}

async fn cogmap_readable(pool: &sqlx::PgPool, profile: Uuid, cogmap: Uuid) -> bool {
    sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn context_visible(pool: &sqlx::PgPool, profile: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_visible_to($1, $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn resource_visible(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn map_in_visible_maps(pool: &sqlx::PgPool, profile: Uuid, cogmap: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cogmap_visible_maps($1) m WHERE m=$2)")
        .bind(profile)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn explicit_cogmap_read_grant_confers_shape_resources_and_wayfind(pool: sqlx::PgPool) {
    let owner = insert_profile(&pool, "rw_owner").await;
    let reader = insert_profile(&pool, "rw_reader").await; // no membership, no grant yet
    let cogmap = fresh_uuid(&pool).await; // synthetic — no membership path involved
    let r = insert_homed_resource(
        &pool,
        "rw-cm-doc",
        "temper://rw-cm",
        "kb_cogmaps",
        cogmap,
        owner,
    )
    .await;

    // Baseline: the reader has no path to the map or its resource.
    assert!(
        !cogmap_readable(&pool, reader, cogmap).await,
        "no shape-read before grant"
    );
    assert!(
        !resource_visible(&pool, reader, r).await,
        "no resource-read before grant"
    );
    assert!(
        !map_in_visible_maps(&pool, reader, cogmap).await,
        "no wayfind admission before grant"
    );

    grant_read(&pool, "kb_cogmaps", cogmap, reader, owner).await;

    // The explicit cogmap read-grant confers all three, coherently.
    assert!(
        cogmap_readable(&pool, reader, cogmap).await,
        "grant ⇒ shape-read"
    );
    assert!(
        resource_visible(&pool, reader, r).await,
        "grant ⇒ homed-resource read"
    );
    assert!(
        map_in_visible_maps(&pool, reader, cogmap).await,
        "grant ⇒ wayfind admission"
    );

    // A different, ungranted profile still sees nothing — additive, no behavior change.
    let other = insert_profile(&pool, "rw_other").await;
    assert!(!cogmap_readable(&pool, other, cogmap).await);
    assert!(!resource_visible(&pool, other, r).await);
    assert!(!map_in_visible_maps(&pool, other, cogmap).await);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn explicit_context_read_grant_confers_context_and_resources(pool: sqlx::PgPool) {
    let owner = insert_profile(&pool, "rw_ctx_owner").await;
    let reader = insert_profile(&pool, "rw_ctx_reader").await;
    let context = fresh_uuid(&pool).await; // synthetic context anchor
    let r = insert_homed_resource(
        &pool,
        "rw-ctx-doc",
        "temper://rw-ctx",
        "kb_contexts",
        context,
        owner,
    )
    .await;

    assert!(
        !context_visible(&pool, reader, context).await,
        "no context-read before grant"
    );
    assert!(
        !resource_visible(&pool, reader, r).await,
        "no resource-read before grant"
    );

    grant_read(&pool, "kb_contexts", context, reader, owner).await;

    assert!(
        context_visible(&pool, reader, context).await,
        "grant ⇒ context-read"
    );
    assert!(
        resource_visible(&pool, reader, r).await,
        "grant ⇒ homed-resource read"
    );

    let other = insert_profile(&pool, "rw_ctx_other").await;
    assert!(!context_visible(&pool, other, context).await);
    assert!(!resource_visible(&pool, other, r).await);
}
