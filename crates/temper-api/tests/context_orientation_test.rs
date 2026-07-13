#![cfg(feature = "test-db")]
//! Context orientation reads (spec §3.7, T8) — `anchor_shape_select` / `anchor_region_metrics_select`
//! over a CONTEXT anchor.
//!
//! The read these prove did not exist before T8, and could not: the orientation trio was keyed on
//! `kb_cogmap_regions.cogmap_id`, which is a FK to `kb_cogmaps` and therefore NULL for every context
//! region. The functions were structurally blind to them — no argument made them return a row.
//!
//! The load-bearing case is `context_read_grant_grants_the_orientation_read`: it is the task's
//! acceptance criterion, and it is the one an inline `EXISTS (… owner …)` gate would fail. The reads
//! gate on `anchor_readable_by_profile` → `context_readable_by_profile` (T1), which consults
//! `kb_access_grants`; a hand-rolled owner-only check would silently deny a legitimate grantee.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ProfileId;
use temper_services::backend::substrate_read::{anchor_region_metrics_select, anchor_shape_select};

mod common;

/// A region homed in `context`, as the real producer writes one: the anchor pair is what the reads are
/// keyed on, and `cogmap_id` is left NULL because a context region cannot carry one (FK to kb_cogmaps).
///
/// `content_cohesion` is deliberately settable as `None` — that is the stored shape of a region whose
/// members are bodyless (zero chunks ⇒ zero-vector centroid), and it is what the reads' `NULLS LAST`
/// exists to keep off the top of a DESC sort.
async fn insert_context_region(
    pool: &PgPool,
    context: Uuid,
    salience: f64,
    cohesion: Option<f64>,
    label: &str,
) -> Uuid {
    let lens: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_cogmap_lenses WHERE name = 'workflow-default'")
            .fetch_one(pool)
            .await
            .expect("the workflow-default lens is seeded by migration");
    // Any committed event satisfies the provenance FKs; the region's provenance is not what is under
    // test here (the read gate is), and the migrations seed the L0 genesis events.
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("migrations seed at least one event");

    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience, centrality,
            content_cohesion, label, member_count, asserted_by_event_id, last_event_id, is_folded)
         VALUES (NULL, 'kb_contexts', $1, $2,
                 array_fill(0::double precision, ARRAY[768])::vector, $3, $4, $5, $6, 3, $7, $7, false)
         RETURNING id",
    )
    .bind(context)
    .bind(lens)
    .bind(salience)
    .bind(salience) // centrality: mirror salience so the metrics sort is deterministic
    .bind(cohesion)
    .bind(label)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert context region")
}

/// Grant a profile explicit READ on a context — the `kb_access_grants` row that
/// `context_readable_by_profile` (T1) consults, and that the pre-T1 inline gate ignored.
async fn grant_context_read(pool: &PgPool, context: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(context)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant context read");
}

/// The owner of a context sees its regions — the read the arc exists to deliver, and which returned
/// nothing (structurally) before T8.
///
/// Doubles as the **differential** for D5's visible-count: the caller can see all three members, so
/// the count they are handed must equal the stored `member_count` exactly. A visible-count that
/// changes a fully-sighted read is a bug in the fix (measured on prod: 0 of 546 live regions diverge).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_sees_the_contexts_regions(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "region-a").await;
    for (i, affinity) in [0.9_f64, 0.5, 0.1].iter().enumerate() {
        let r = insert_resource(&pool, context, profile, &format!("member-{i}")).await;
        add_member(&pool, region, r, *affinity).await;
    }

    let rows = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(context.into()),
        None,
    )
    .await
    .expect("context shape read must be Ok");

    assert_eq!(rows.len(), 1, "the context's one region surfaces: {rows:?}");
    assert_eq!(rows[0].label.as_deref(), Some("region-a"));
    assert_eq!(
        rows[0].member_count, 3,
        "a caller who can see every member is handed the stored count, unchanged"
    );
}

/// THE ACCEPTANCE CRITERION: "a context read-grant actually grants access to the orientation read."
///
/// A stranger — no ownership, no team reach — sees nothing. Give that same stranger an explicit
/// `kb_access_grants` READ row on the context, and the identical call now returns the regions. This is
/// what gating on `context_readable_by_profile` (T1) buys; an owner-only `EXISTS` would deny them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_read_grant_grants_the_orientation_read(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    let stranger = common::fixtures::create_test_profile(&pool, "stranger@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "region-a").await;
    // The region needs a member the grantee can see: the context READ grant is what makes the
    // context's own resources visible (`resources_visible_to` → `contexts_readable_by`), so the grant
    // carries both halves — the anchor gate AND the members it is counted over.
    let member = insert_resource(&pool, context, owner, "a resource in the granted context").await;
    add_member(&pool, region, member, 0.9).await;

    let anchor = HomeAnchor::Context(context.into());

    // Before the grant: zero rows — and NOT an error. The gate is in the SQL, so a denied principal
    // cannot distinguish "not readable" from "no regions" (no existence oracle).
    let before = anchor_shape_select(&pool, ProfileId::from(stranger), anchor, None)
        .await
        .expect("a denied read is empty, never an error");
    assert!(
        before.is_empty(),
        "a stranger must not see the context's regions: {before:?}"
    );

    grant_context_read(&pool, context, stranger).await;

    // After the grant: the same call, the same principal, now returns the regions.
    let after = anchor_shape_select(&pool, ProfileId::from(stranger), anchor, None)
        .await
        .expect("granted read must be Ok");
    assert_eq!(
        after.len(),
        1,
        "a context READ grant must grant the orientation read: {after:?}"
    );
    assert_eq!(after[0].label.as_deref(), Some("region-a"));

    // The analytics tier is gated by the same predicate, so the grant must carry it too — otherwise
    // the two reads would disagree about who may look at the same context.
    let metrics = anchor_region_metrics_select(&pool, ProfileId::from(stranger), anchor, None)
        .await
        .expect("granted metrics read must be Ok");
    assert_eq!(
        metrics.len(),
        1,
        "the grant must carry the analytics tier too: {metrics:?}"
    );
}

/// Rows come back most-salient-first, and a NULL `content_cohesion` does not hijack the top.
///
/// This is the NULL cousin of T7's NaN trap. A region whose members are bodyless stores NULL cohesion
/// (11 such regions exist in prod), and Postgres sorts NULL **first** on `ORDER BY … DESC` — so
/// without `NULLS LAST` the contentless region would lead every orientation read, exactly as the
/// zero-centroid regions led every wayfind before T7 guarded them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn regions_sort_most_salient_first_and_nulls_do_not_lead(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    for (salience, cohesion, label) in [
        (0.2, Some(0.9), "low-salience"),
        (0.8, None, "high-salience-no-cohesion"),
        (0.5, Some(0.4), "mid-salience"),
    ] {
        let region = insert_context_region(&pool, context, salience, cohesion, label).await;
        // Every region needs at least one VISIBLE member to be returned at all (D5): a region the
        // caller can see nothing in is not a region they can see. Sorting is what's under test here.
        let r = insert_resource(&pool, context, profile, &format!("{label}-member")).await;
        add_member(&pool, region, r, 0.9).await;
    }

    let rows = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(context.into()),
        None,
    )
    .await
    .expect("context shape read must be Ok");

    let labels: Vec<_> = rows.iter().filter_map(|r| r.label.as_deref()).collect();
    assert_eq!(
        labels,
        vec!["high-salience-no-cohesion", "mid-salience", "low-salience"],
        "most salient first; a NULL cohesion neither leads nor is dropped"
    );
}

/// A context the caller cannot read yields empty, never an error — the same leak-safe shape the cogmap
/// reads have. (A random UUID stands in for "a context that exists but is not yours": both are denied
/// identically, which is the point — the caller learns nothing about existence.)
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_context_is_empty_not_error(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let rows = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(Uuid::now_v7().into()),
        None,
    )
    .await
    .expect("non-readable context is empty, not an error");
    assert!(rows.is_empty());
}

// ── The label fallback (T8 follow-up, migration 20260713000020) ──────────────
//
// `kb_cogmap_regions.label` is NULL for 100% of live regions on prod — 0 of 276 context regions AND
// 0 of 251 cogmap regions. The producer never writes it. So the orientation read, whose entire job is
// to answer "what is this context about", was returning anonymous UUIDs. `anchor_shape` now falls
// back to the most-affine VISIBLE member's title (parity with `graph_cogmap_territories`).

/// A resource, homed in `context` and owned by `owner`.
async fn insert_resource(pool: &PgPool, context: Uuid, owner: Uuid, title: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id",
    )
    .bind(title)
    .fetch_one(pool)
    .await
    .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(context)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home resource");
    id
}

async fn add_member(pool: &PgPool, region: Uuid, resource: Uuid, affinity: f64) {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
         VALUES ($1, 'kb_resources', $2, $3)",
    )
    .bind(region)
    .bind(resource)
    .bind(affinity)
    .execute(pool)
    .await
    .expect("add region member");
}

/// An unlabelled region takes its name from its most-affine member — the difference between a UUID
/// and an answer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unlabelled_region_is_named_by_its_most_affine_member(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "").await;
    // Clear the label so the region is genuinely unlabelled, as every real region is.
    sqlx::query("UPDATE kb_cogmap_regions SET label = NULL WHERE id = $1")
        .bind(region)
        .execute(&pool)
        .await
        .unwrap();

    let minor = insert_resource(&pool, context, profile, "a peripheral note").await;
    let central = insert_resource(&pool, context, profile, "Deployment & Release Workflow").await;
    add_member(&pool, region, minor, 0.2).await;
    add_member(&pool, region, central, 0.9).await;

    let rows = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(context.into()),
        None,
    )
    .await
    .expect("shape read must be Ok");

    assert_eq!(
        rows[0].label.as_deref(),
        Some("Deployment & Release Workflow"),
        "an unlabelled region is named by its MOST-AFFINE member, not just any member"
    );
}

/// THE LEAK TEST. A region can legitimately contain a resource the caller cannot read — region
/// membership is not resource visibility. Surfacing that resource's title as the region's label would
/// leak it through a read whose own gate says nothing about members.
///
/// Here the *most affine* member is invisible to the caller and a *less* affine one is visible: the
/// label must be the visible one. An un-gated `ORDER BY affinity DESC LIMIT 1` would name the region
/// after the secret and this test would catch it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_invisible_member_can_never_become_the_regions_name(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    // A second profile with its own context — nothing there is visible to `profile`.
    let (stranger, stranger_context) =
        common::fixtures::create_test_profile_with_context(&pool, "stranger@example.com").await;

    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "").await;
    sqlx::query("UPDATE kb_cogmap_regions SET label = NULL WHERE id = $1")
        .bind(region)
        .execute(&pool)
        .await
        .unwrap();

    // The most-affine member is a resource the caller CANNOT see.
    let secret = insert_resource(
        &pool,
        stranger_context,
        stranger,
        "CONFIDENTIAL acquisition memo",
    )
    .await;
    let visible = insert_resource(&pool, context, profile, "Deployment & Release Workflow").await;
    add_member(&pool, region, secret, 0.99).await; // highest affinity
    add_member(&pool, region, visible, 0.30).await;

    let rows = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(context.into()),
        None,
    )
    .await
    .expect("shape read must be Ok");

    let label = rows[0].label.as_deref();
    assert_ne!(
        label,
        Some("CONFIDENTIAL acquisition memo"),
        "a member the caller cannot read must NEVER become the region's name"
    );
    assert_eq!(
        label,
        Some("Deployment & Release Workflow"),
        "the name falls to the most-affine VISIBLE member"
    );

    // ...and D5: having declined to NAME the invisible member, we must not COUNT it either. The region
    // stores `member_count = 3` and holds two members, exactly one of which this caller can read. The
    // honest answer is 1. Anything else is a cardinality disclosure about content they have no read on.
    assert_eq!(
        rows[0].member_count, 1,
        "the count is over VISIBLE members only — not the stored count, not the member rows"
    );
}

/// A region the caller can see NOTHING in is not a region they can see — at EITHER door.
///
/// The shape read and the metrics read enumerate the same regions off the same anchor. If the shape
/// read hides a region while the metrics read still answers for it, the metrics door becomes an
/// existence oracle for exactly the regions the shape door refuses to show — and hands over its
/// centrality and cohesion besides. Both doors drop it, or neither is closed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_region_with_no_visible_members_is_returned_by_neither_door(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    let (stranger, stranger_context) =
        common::fixtures::create_test_profile_with_context(&pool, "stranger@example.com").await;

    // A region on the caller's OWN context — the anchor gate passes — whose every member lives
    // somewhere they cannot read. The anchor says yes; the members say there is nothing to see.
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "all-invisible").await;
    let secret_a = insert_resource(&pool, stranger_context, stranger, "secret one").await;
    let secret_b = insert_resource(&pool, stranger_context, stranger, "secret two").await;
    add_member(&pool, region, secret_a, 0.9).await;
    add_member(&pool, region, secret_b, 0.4).await;

    let anchor = HomeAnchor::Context(context.into());

    let shape = anchor_shape_select(&pool, ProfileId::from(profile), anchor, None)
        .await
        .expect("shape read must be Ok");
    assert!(
        shape.is_empty(),
        "a region with no visible members must not surface in the shape read: {shape:?}"
    );

    let metrics = anchor_region_metrics_select(&pool, ProfileId::from(profile), anchor, None)
        .await
        .expect("metrics read must be Ok");
    assert!(
        metrics.is_empty(),
        "...nor in the metrics read, which would otherwise answer for a region the shape read hides: \
         {metrics:?}"
    );
}

/// A SOFT-DELETED member is not a member. This is the arm of D5 that bites TODAY, on every caller
/// including the owner: `resources_visible_to` declares a deleted resource "invisible on every axis",
/// yet the stored `member_count` — written at materialize time — kept counting it. So a region whose
/// member was deleted reported a count including a resource that no longer exists to anyone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_soft_deleted_member_is_not_counted(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "region-a").await;

    let live = insert_resource(&pool, context, profile, "still here").await;
    let deleted = insert_resource(&pool, context, profile, "deleted since materialize").await;
    add_member(&pool, region, live, 0.5).await;
    add_member(&pool, region, deleted, 0.9).await; // the MOST affine member, and it is gone

    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(deleted)
        .execute(&pool)
        .await
        .expect("soft-delete the member");

    let rows = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(context.into()),
        None,
    )
    .await
    .expect("shape read must be Ok");

    assert_eq!(
        rows[0].member_count, 1,
        "the deleted member is not counted — even for the owner, who could see it when it existed"
    );
}
