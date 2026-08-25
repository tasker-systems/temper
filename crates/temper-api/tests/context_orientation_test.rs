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

use temper_core::types::cognitive_maps::ShapeEmptiness;
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

/// A SECOND context owned by an existing profile. `create_test_profile_with_context` mints exactly
/// one (slug `temper`) and the slug is unique per owner, so a sibling context — the thing a reader
/// can read but a *grantee on another context* cannot — has to be inserted here.
async fn insert_owned_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("insert a second owned context")
}

/// Stamp the FORMATION watermark the envelope's clock reads. A fixture context is born with
/// `shape_materialized_event_id` NULL, so it reports `never_clustered` until this runs — and the
/// emptiness precedence checks the clock BEFORE the row count
/// (`migrations/20260823000010_anchor_shape_envelope.sql:90-92`), which is exactly why a test that
/// means to prove `nothing_visible` must materialize first or it proves `never_clustered` instead.
async fn mark_context_materialized(pool: &PgPool, context: Uuid) {
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("migrations seed at least one event");
    sqlx::query("UPDATE kb_contexts SET shape_materialized_event_id = $2 WHERE id = $1")
        .bind(context)
        .bind(event)
        .execute(pool)
        .await
        .expect("stamp the context's shape watermark");
}

/// The id of a boot-seeded GLOBAL lens (`cogmap_id IS NULL`). Two exist — `workflow-default`
/// (`migrations/20260712000050_workflow_default_lens.sql:107`) and `telos-default` — which is what
/// makes a two-lens fixture possible without minting one.
async fn global_lens(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM kb_cogmap_lenses WHERE name = $1 AND cogmap_id IS NULL",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("the global lens is seeded by migration")
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
    .expect("context shape read must be Ok")
    .regions;

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
        .expect("a denied read is empty, never an error")
        .regions;
    assert!(
        before.is_empty(),
        "a stranger must not see the context's regions: {before:?}"
    );

    grant_context_read(&pool, context, stranger).await;

    // After the grant: the same call, the same principal, now returns the regions.
    let after = anchor_shape_select(&pool, ProfileId::from(stranger), anchor, None)
        .await
        .expect("granted read must be Ok")
        .regions;
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
    .expect("context shape read must be Ok")
    .regions;

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
///
/// The envelope keeps that promise rather than weakening it. `unreadable_or_absent` is ONE arm for
/// both situations, and it reports neither the population nor the clock — so what the caller now
/// learns is that they are in {denied, absent}, which is strictly narrower than the `[]` they used to
/// get and which they already knew. A `population: 0` here would still be a fact about a context they
/// have no read on, so the assertions below pin both fields, not just the row count.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_context_is_empty_not_error(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let shape = anchor_shape_select(
        &pool,
        ProfileId::from(profile),
        HomeAnchor::Context(Uuid::now_v7().into()),
        None,
    )
    .await
    .expect("non-readable context is empty, not an error");

    assert!(shape.regions.is_empty());
    assert_eq!(
        shape.emptiness,
        Some(ShapeEmptiness::UnreadableOrAbsent),
        "deny and absent collapse into one arm: {shape:?}"
    );
    assert_eq!(
        shape.population, 0,
        "the arm must not disclose the size of a context the caller cannot read"
    );
    assert!(
        shape.materialized_at.is_none(),
        "...nor its clock, which would confirm the context exists and has been clustered"
    );
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
    .expect("shape read must be Ok")
    .regions;

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
    .expect("shape read must be Ok")
    .regions;

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
///
/// This is also the canonical `nothing_visible` case: the anchor HAS been clustered and does hold a
/// region, and the caller still gets nothing — which before the envelope was byte-identical to an
/// anchor that had never clustered at all. The context is materialized here on purpose; without that
/// stamp the emptiness precedence answers `never_clustered` first
/// (`migrations/20260823000010_anchor_shape_envelope.sql:90-92`) and the arm this test names would
/// never be reached.
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
    mark_context_materialized(&pool, context).await;

    let anchor = HomeAnchor::Context(context.into());

    let shape = anchor_shape_select(&pool, ProfileId::from(profile), anchor, None)
        .await
        .expect("shape read must be Ok");
    assert!(
        shape.regions.is_empty(),
        "a region with no visible members must not surface in the shape read: {shape:?}"
    );
    assert_eq!(
        shape.emptiness,
        Some(ShapeEmptiness::NothingVisible),
        "clustered, and holding a region — the emptiness is about REACH, not about formation: \
         {shape:?}"
    );
    assert_eq!(
        shape.population, 0,
        "the denominator is member-gated too: a region the caller can see nothing in is not counted"
    );
    assert!(
        shape.materialized_at.is_some(),
        "the clock IS disclosed to a caller who passed the anchor gate — that is what separates this \
         arm from unreadable_or_absent"
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
    .expect("shape read must be Ok")
    .regions;

    assert_eq!(
        rows[0].member_count, 1,
        "the deleted member is not counted — even for the owner, who could see it when it existed"
    );
}

// ── The anchor-level envelope (migration 20260823000010) ─────────────────────
//
// Before it, `[]` was the whole answer, and four different situations produced identical bytes. The
// two tests below cover the pair the read could not separate, and the field that makes `population`
// a denominator rather than a restatement of `regions.len()`.

/// THE PAIR THE OLD READ COULD NOT SEPARATE. Both contexts answer with zero regions; only the
/// envelope says why. `never_clustered` has never been materialized — there is nothing to see
/// because nothing was ever formed. `clustered` has been materialized and holds a region — there is
/// nothing to see because this caller can reach none of its members. The CLI help text asserted the
/// first cause for both (`crates/temper-cli/src/cli.rs:1079`), which is the claim this ends.
///
/// Both contexts are owned by the SAME reader, so the anchor gate answers identically for each and
/// the only variables are formation and reach.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_never_clustered_context_is_distinguishable_from_one_with_nothing_visible(pool: PgPool) {
    let (reader, never_clustered) =
        common::fixtures::create_test_profile_with_context(&pool, "reader@example.com").await;
    let (stranger, stranger_context) =
        common::fixtures::create_test_profile_with_context(&pool, "stranger@example.com").await;

    let clustered = insert_owned_context(&pool, reader, "clustered").await;
    let region = insert_context_region(&pool, clustered, 0.9, Some(0.5), "all-invisible").await;
    let secret = insert_resource(&pool, stranger_context, stranger, "not yours").await;
    add_member(&pool, region, secret, 0.9).await;
    mark_context_materialized(&pool, clustered).await;

    let a = anchor_shape_select(
        &pool,
        ProfileId::from(reader),
        HomeAnchor::Context(never_clustered.into()),
        None,
    )
    .await
    .expect("a readable context read must be Ok");
    let b = anchor_shape_select(
        &pool,
        ProfileId::from(reader),
        HomeAnchor::Context(clustered.into()),
        None,
    )
    .await
    .expect("a readable context read must be Ok");

    assert!(
        a.regions.is_empty() && b.regions.is_empty(),
        "both are empty — exactly as they were before the envelope: {a:?} / {b:?}"
    );
    assert_eq!(a.emptiness, Some(ShapeEmptiness::NeverClustered));
    assert_eq!(b.emptiness, Some(ShapeEmptiness::NothingVisible));
    assert_ne!(
        a.emptiness, b.emptiness,
        "the two byte-identical answers now differ — this is the whole point of the task"
    );
    assert!(
        a.materialized_at.is_none() && b.materialized_at.is_some(),
        "and the clock agrees with the arm each one named: {a:?} / {b:?}"
    );
}

/// The task's first acceptance criterion: two principals with different reach receive different
/// populations.
///
/// Asserted UNDER A LENS on purpose. Without one, `population` equals `regions.len()` and the
/// criterion is met by the row count alone — which is the premise the design's §2.1 records as
/// unsupported by disk. Under a lens the two callers are handed the SAME single row, so the rows
/// cannot be what differs; only the all-lens denominator can be. Reach decides it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn population_is_member_gated_across_two_principals(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "owner@example.com").await;
    let grantee = common::fixtures::create_test_profile(&pool, "grantee@example.com").await;
    grant_context_read(&pool, context, grantee).await;
    mark_context_materialized(&pool, context).await;

    let lens_a = global_lens(&pool, "workflow-default").await;
    let lens_b = global_lens(&pool, "telos-default").await;

    // Region under lens A, holding a resource homed in the shared context — visible to BOTH: the
    // context READ grant is what makes the context's own resources visible to the grantee.
    let shared_region = insert_context_region(&pool, context, 0.9, Some(0.5), "shared").await;
    let shared_member = insert_resource(&pool, context, owner, "in the granted context").await;
    add_member(&pool, shared_region, shared_member, 0.9).await;

    // Region under lens B, holding a resource homed in a SECOND context of the owner's. The grant
    // covers one context, not the owner's whole reach, so this member is invisible to the grantee —
    // and the region therefore falls out of THEIR denominator, though not out of the owner's.
    let private = insert_owned_context(&pool, owner, "private").await;
    let private_region = insert_context_region(&pool, context, 0.4, Some(0.5), "private").await;
    // `insert_context_region` always writes the workflow-default lens; the second region is moved to
    // the other global lens here rather than by forking the helper (the same in-test UPDATE the
    // label-fallback cases use to reshape a seeded region).
    sqlx::query("UPDATE kb_cogmap_regions SET lens_id = $2 WHERE id = $1")
        .bind(private_region)
        .bind(lens_b)
        .execute(&pool)
        .await
        .expect("move the second region under the other global lens");
    let private_member = insert_resource(&pool, private, owner, "not in the granted context").await;
    add_member(&pool, private_region, private_member, 0.9).await;

    let anchor = HomeAnchor::Context(context.into());
    let wide = anchor_shape_select(&pool, ProfileId::from(owner), anchor, Some(lens_a))
        .await
        .expect("owner read must be Ok");
    let narrow = anchor_shape_select(&pool, ProfileId::from(grantee), anchor, Some(lens_a))
        .await
        .expect("grantee read must be Ok");

    assert_eq!(
        (wide.regions.len(), narrow.regions.len()),
        (1, 1),
        "the lens narrows both callers to the same single row: {wide:?} / {narrow:?}"
    );
    assert_eq!(
        wide.population, 2,
        "the owner reaches a member in each region, so both count: {wide:?}"
    );
    assert_eq!(
        narrow.population, 1,
        "the grantee reaches a member in only one of them: {narrow:?}"
    );
    assert!(
        wide.population > narrow.population,
        "reach decides the denominator, not just the rows"
    );
    assert_eq!(
        wide.emptiness, None,
        "a non-empty row set carries no emptiness at all"
    );
}

// ---------------------------------------------------------------------------------------------
// The clock (`anchor_staleness`, migrations/20260823000020) — the same generalization as the reads
// above, applied to the staleness aggregate. `cogmap_staleness` keyed its regions arm on
// `kb_cogmap_regions.cogmap_id` (20260624000002:540), the FK that is NULL for every context region,
// so it was structurally blind to them in exactly the way the orientation reads were before T8.
// ---------------------------------------------------------------------------------------------

/// Mint an event that occurred strictly AFTER the fixture's watermark.
///
/// The timestamp is explicit (`now() + interval '1 day'`) rather than the column default:
/// `mark_context_materialized` stamps a migration-seeded event whose `occurred_at` is only
/// microseconds behind the test's own clock, and every assertion in this arc turns on
/// `latest_touch > materialized_at` being unambiguously true rather than on two stamps that tie.
async fn mint_event_later_than_the_watermark(pool: &PgPool, profile: Uuid) -> Uuid {
    // A fresh emitter entity, as `common::seed_genesis_event` does: `kb_events.emitter_entity_id`
    // is NOT NULL and the entity name is unique, so it is suffixed with the id.
    let entity = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(entity)
        .bind(profile)
        .bind(format!("toucher-{entity}@web"))
        .execute(pool)
        .await
        .expect("insert the touching emitter entity");

    sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, occurred_at) \
         SELECT (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'), $1, \
                now() + interval '1 day' \
         RETURNING id",
    )
    .bind(entity)
    .fetch_one(pool)
    .await
    .expect("mint an event later than the watermark")
}

/// Point one region's `last_event_id` at an event later than the watermark — the "touched since it
/// materialized" half of the witnesses below.
async fn touch_region_with_a_later_event(pool: &PgPool, profile: Uuid, region: Uuid) {
    let event = mint_event_later_than_the_watermark(pool, profile).await;
    sqlx::query("UPDATE kb_cogmap_regions SET last_event_id = $2 WHERE id = $1")
        .bind(region)
        .bind(event)
        .execute(pool)
        .await
        .expect("advance the region's clock");
}

/// The edges-arm counterpart of `touch_region_with_a_later_event`.
async fn touch_edge_with_a_later_event(pool: &PgPool, profile: Uuid, edge: Uuid) {
    let event = mint_event_later_than_the_watermark(pool, profile).await;
    sqlx::query("UPDATE kb_edges SET last_event_id = $2 WHERE id = $1")
        .bind(edge)
        .bind(event)
        .execute(pool)
        .await
        .expect("advance the edge's clock");
}

/// The event the context's `shape_materialized_event_id` points at — read back rather than
/// re-derived with the `ORDER BY occurred_at LIMIT 1` that `mark_context_materialized` uses, so an
/// edge born "at the watermark" is born at THE watermark and not merely at something that ties with
/// it.
async fn context_watermark_event(pool: &PgPool, context: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1")
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("the context must be materialized before an edge can be born at its watermark")
}

/// An edge homed on a CONTEXT, between two `kb_resources` endpoints, born at that context's own
/// shape watermark — so it starts out contributing a `latest_touch` exactly EQUAL to
/// `materialized_at` (`>` is false, the anchor reads fresh) until something moves it.
///
/// The three table literals are the only values the CHECK constraints admit here:
/// `kb_edges_home_anchor_table_check` restricts the home to `{kb_contexts, kb_cogmaps}` and the two
/// endpoint checks restrict both endpoints to `{kb_resources, kb_cogmaps}`. That is what makes
/// `endpoint_readable_by_profile`'s `ELSE false` arm unreachable for any real edge — the constraint
/// argument `migrations/20260825000010_staleness_member_gate.sql:232-239` makes to establish that
/// the gated edges arm can only ever drop an edge for a VISIBILITY reason.
async fn insert_context_edge(pool: &PgPool, context: Uuid, source: Uuid, target: Uuid) -> Uuid {
    let event = context_watermark_event(pool, context).await;
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO kb_edges \
           (source_table, source_id, target_table, target_id, edge_kind, \
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, 'express', \
                 'kb_contexts', $3, $4, $4, false) \
         RETURNING id",
    )
    .bind(source)
    .bind(target)
    .bind(context)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert a context-homed edge")
}

/// The soft-delete floor, applied. `resources_visible_to` ends in
/// `JOIN kb_resources r ON r.id = v.resource_id AND r.is_active`, so this one flag makes the
/// resource invisible on every axis — INCLUDING to the profile that owns it, which is what lets the
/// ghost fixtures below bite the owner and need no second principal at all.
async fn soft_delete_resource(pool: &PgPool, resource: Uuid) {
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(resource)
        .execute(pool)
        .await
        .expect("soft-delete the resource");
}

/// One `anchor_staleness` row, decoded as BOOLEANS rather than timestamps.
///
/// Every field is computed in SQL so that nothing here depends on a timestamp decoder:
/// `temper-api`'s **dev**-dependency on sqlx does not enable the `chrono` feature
/// (`crates/temper-api/Cargo.toml`, `[dev-dependencies] sqlx`), and leaning on feature unification
/// with the lib dependency to supply one would be an invisible coupling for a test file to carry.
#[derive(Debug, Clone, Copy)]
struct Staleness {
    is_stale: bool,
    has_materialized_at: bool,
    has_latest_touch: bool,
    /// `latest_touch = materialized_at`. The discriminator `has_latest_touch` alone cannot supply:
    /// it separates "the arm is gated and contributed nothing" from "the arm is WORKING and what it
    /// contributed simply has not moved past the watermark". A test whose fixture holds one readable
    /// and one unreadable row on the same arm needs that distinction, or a mutation that deletes the
    /// arm outright would pass it.
    touch_equals_watermark: bool,
}

/// One `anchor_staleness` row for `p_principal_kind = 'profile'`, or `None` when the function yields
/// ZERO ROWS.
///
/// **`Option` is the point of this helper, not defensiveness.** After
/// `migrations/20260825000010_staleness_member_gate.sql` the deny path IS zero rows (`:249-254`) —
/// deliberately indistinguishable from an absent anchor — so a helper that `fetch_one`s cannot
/// express the answer the deny test has to assert, and would report a gate failure as a decode
/// panic.
///
/// Runtime `query_as`, not the macro, for the reason the incumbent helper gave and which the new
/// signature does not change: these functions are new in this migration and a compile-time-checked
/// call would demand a `.sqlx` cache entry that only `cargo sqlx prepare` against a migrated
/// database can produce. `is_stale` is read as `Option<bool>` because `RETURNS TABLE` declares it
/// nullable even though the `COALESCE` never yields NULL — a NULL here would itself be a finding, so
/// it is unwrapped with a message rather than decoded into `bool`.
async fn anchor_staleness_row(
    pool: &PgPool,
    anchor_table: &str,
    anchor_id: Uuid,
    profile: Uuid,
) -> Option<Staleness> {
    let row: Option<(Option<bool>, bool, bool, bool)> = sqlx::query_as(
        "SELECT s.is_stale, \
                s.materialized_at IS NOT NULL, \
                s.latest_touch IS NOT NULL, \
                COALESCE(s.latest_touch = s.materialized_at, false) \
           FROM anchor_staleness($1, $2, 'profile', $3) s",
    )
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .expect("anchor_staleness is never an error — deny and absence are both zero rows");

    row.map(|(stale, has_mat, has_touch, equal)| Staleness {
        is_stale: stale.expect("is_stale is a COALESCE and is never NULL"),
        has_materialized_at: has_mat,
        has_latest_touch: has_touch,
        touch_equals_watermark: equal,
    })
}

/// `anchor_staleness` over a context that the caller is expected to be able to read — the common
/// case, where zero rows would itself be the failure and is reported as such.
async fn context_staleness(pool: &PgPool, context: Uuid, profile: Uuid) -> Staleness {
    anchor_staleness_row(pool, "kb_contexts", context, profile)
        .await
        .expect("a readable context that exists yields exactly one anchor_staleness row")
}

/// **The witness for the generalized clock**, and the only assertion in this arc that can tell the
/// working function from the broken one.
///
/// The trap it exists to catch is silent. If `anchor_staleness`' regions arm is left keyed on
/// `reg.cogmap_id` while the signature is generalized, a context does not error and does not return
/// NULLs: `latest_touch` comes back NULL, `latest_touch > materialized_at` is therefore NULL, and
/// the `COALESCE` falls through to `materialized_at IS NULL` — **false** for any context that has
/// materialized even once. Every context would report `is_stale = false` forever and nothing would
/// go red.
///
/// So a fixture that only materializes proves nothing: it reports `is_stale = false` under both the
/// working and the broken function. This one materializes AND THEN touches a region with a later
/// event, which is the only shape that separates them. The `has_touch` assertion on the fresh read
/// is a second, independent discriminator: under the broken arm `latest_touch` is NULL there too.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_touched_context_reports_stale(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "clock@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "the region").await;
    // The region needs ONE VISIBLE MEMBER or it does not move this clock at all
    // (`migrations/20260825000010_staleness_member_gate.sql:206-212`) — the same rule both
    // enumeration doors already applied, now applied to the clock. Before that migration this
    // fixture held a member-less region and still reported a touch, which is precisely the
    // disclosure `a_ghost_region_does_not_move_the_staleness_clock` below pins.
    let member = insert_resource(&pool, context, profile, "a live member").await;
    add_member(&pool, region, member, 0.9).await;
    // `insert_context_region` stamps the region's clock with the earliest seeded event and
    // `mark_context_materialized` stamps the watermark with the same one, so the context starts
    // materialized-and-untouched: the two timestamps are equal and `>` is false.
    mark_context_materialized(&pool, context).await;

    let s = context_staleness(&pool, context, profile).await;
    assert!(s.has_materialized_at, "the context was just materialized");
    assert!(
        s.has_latest_touch,
        "latest_touch is NULL only if the regions arm cannot see this context's regions — i.e. it \
         is still keyed on the vestigial cogmap_id instead of the anchor pair"
    );
    assert!(
        !s.is_stale,
        "nothing has touched the context since its watermark, so it is fresh"
    );

    touch_region_with_a_later_event(&pool, profile, region).await;

    let s = context_staleness(&pool, context, profile).await;
    assert!(
        s.has_materialized_at && s.has_latest_touch,
        "both clocks are still readable"
    );
    assert!(
        s.is_stale,
        "a context touched after materializing is stale — if this is false, the regions arm is \
         still keyed on cogmap_id and every context is permanently, silently fresh"
    );
}

/// A context that has never been materialized is stale — the `materialized_at IS NULL` limb of the
/// `COALESCE`, carried over unchanged from `cogmap_staleness` (20260624000002:549).
///
/// Pins the limb the trap hides behind: it is the arm that answers when `latest_touch >
/// materialized_at` is NULL, and it is the reason the broken function returns a plausible `false`
/// rather than an obvious NULL.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_never_materialized_context_is_stale(pool: PgPool) {
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "unclustered@example.com").await;
    let region = insert_context_region(&pool, context, 0.5, None, "the region").await;
    // A visible member, so the region reaches the clock at all under the member gate. Not strictly
    // needed for the limb under test — `materialized_at IS NULL` is true whatever `latest_touch`
    // says — but a member-less region would leave this test silently exercising the gated-out path
    // rather than the limb it names.
    let member = insert_resource(&pool, context, profile, "a live member").await;
    add_member(&pool, region, member, 0.9).await;

    let s = context_staleness(&pool, context, profile).await;
    assert!(
        !s.has_materialized_at,
        "the fixture context carries no watermark yet"
    );
    assert!(
        s.is_stale,
        "never materialized reads as stale, not as fresh"
    );
}

/// A context that does not exist yields ZERO rows, not a row of NULLs — the behaviour
/// `cogmap_analytics` already depends on ("cogmap_staleness yields exactly one row",
/// `migrations/20260628000001_cogmap_analytics_read_functions.sql:25-26`), which is what makes its
/// gate in `WHERE` deny to an empty result rather than to a spurious row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_absent_anchor_yields_no_row(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let got = anchor_staleness_row(&pool, "kb_contexts", Uuid::now_v7(), profile).await;

    assert!(
        got.is_none(),
        "an anchor that does not exist has no clock to report: {got:?}"
    );
}

/// A cogmap whose shape watermark is stamped, plus ONE region homed on it exactly as the production
/// writer writes one — `cogmap_id` AND the anchor pair, set together in one INSERT
/// (`crates/temper-substrate/src/write.rs:688-696`). Returns `(cogmap, region)`.
///
/// Setting both is the whole point of the fixture and not incidental detail. `kb_cogmap_regions` has
/// no CHECK tying `cogmap_id` to `(home_anchor_table, home_anchor_id)`, so the equality the
/// delegation rests on is convention and backfill only
/// (`migrations/20260823000020_anchor_staleness.sql`, the comment above `cogmap_staleness`). A
/// fixture that bound `cogmap_id` alone would construct a row the real system never produces, and
/// the wrapper would answer `is_stale = false` on it forever.
///
/// **Two things this fixture gained with `20260825000010`, and neither is decoration.** The map is
/// now linked to a team `profile` can reach and its region is given a member homed in the map,
/// because the wrapper is gated from that migration onward: `cogmap_readable_by_profile` admits a
/// map only through `kb_team_cogmaps` ⋈ `profile_reachable_teams` or an explicit grant, and the
/// regions arm now requires the region to hold a member the caller can see. A bare map with an empty
/// region — what this fixture used to build — would answer ZERO ROWS, and the delegation test would
/// fail for a reason that has nothing to do with the delegation.
async fn insert_materialized_cogmap_with_region(
    pool: &PgPool,
    profile: Uuid,
    name: &str,
) -> (Uuid, Uuid) {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .bind(format!("test://{name}-telos"))
    .fetch_one(pool)
    .await
    .expect("insert the cogmap's telos resource");

    // The same earliest seeded event serves as both the map's watermark and the region's birth
    // clock, so the map starts materialized-and-untouched: the two stamps are equal and `>` is false.
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY occurred_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("migrations seed at least one event");

    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id, shape_materialized_event_id) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert a materialized cogmap");

    let lens = global_lens(pool, "telos-default").await;
    let region: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, centroid, salience, centrality,
            content_cohesion, label, member_count, asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, 'kb_cogmaps', $1, $2,
                 array_fill(0::double precision, ARRAY[768])::vector, 0.7, 0.7, 0.5, 'the region',
                 2, $3, $3, false)
         RETURNING id",
    )
    .bind(cogmap)
    .bind(lens)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert a cogmap-homed region carrying BOTH keys");

    // The gate's anchor half: `cogmap_readable_by_profile` admits a map only through
    // `kb_team_cogmaps` joined to `profile_reachable_teams`, or an explicit grant.
    //
    // `profile_effective_teams` and NOT `profile_reachable_teams`, deliberately. The first is DIRECT
    // memberships (`kb_team_members` ⋈ active teams), and a fresh test profile has exactly one — the
    // `personal-<handle>` team minted by `sync_personal_team`. The second expands UP through
    // `team_ancestors`, and that same trigger parents every personal team to `temper-system`, so
    // `reachable` holds at least two rows in no defined order — a `LIMIT 1` over it could bind this
    // map to the ROOT team, which would make it readable by every profile in the database.
    let team: Uuid = sqlx::query_scalar("SELECT team_id FROM profile_effective_teams($1)")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("the personal-team trigger gives every test profile exactly one direct team");
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(pool)
        .await
        .expect("link the map to a team the profile can reach");

    // The gate's member half: a resource homed IN the map, which the same team link makes visible
    // (the "resources homed in a cognitive map joined to a REACHABLE team" arm of
    // `resources_visible_to`), added to the region so it is a region the caller can see into.
    let member: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("{name}-member"))
    .bind(format!("test://{name}-member"))
    .fetch_one(pool)
    .await
    .expect("insert the region's member resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(member)
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home the member in the map");
    add_member(pool, region, member, 0.9).await;

    (cogmap, region)
}

/// One `cogmap_staleness` row — the DELEGATING wrapper, not `anchor_staleness` — or `None` on the
/// wrapper's zero-row deny.
///
/// Deliberately calls the wrapper by its own name. That name is what `cogmap_analytics`
/// (`migrations/20260628000001_cogmap_analytics_read_functions.sql:37`) and the scenario runner
/// (`crates/temper-substrate/src/scenario/runner.rs:486`) call, so it is the surface whose answer
/// must not move; calling `anchor_staleness('kb_cogmaps', …)` here would test the delegate and skip
/// the delegation. The wrapper carries NO gate of its own — it inherits the core's — which is why
/// this returns an `Option` too.
async fn cogmap_staleness_row(pool: &PgPool, cogmap: Uuid, profile: Uuid) -> Option<Staleness> {
    let row: Option<(Option<bool>, bool, bool, bool)> = sqlx::query_as(
        "SELECT s.is_stale, \
                s.materialized_at IS NOT NULL, \
                s.latest_touch IS NOT NULL, \
                COALESCE(s.latest_touch = s.materialized_at, false) \
           FROM cogmap_staleness($1, 'profile', $2) s",
    )
    .bind(cogmap)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .expect("cogmap_staleness is never an error — deny and absence are both zero rows");

    row.map(|(stale, has_mat, has_touch, equal)| Staleness {
        is_stale: stale.expect("is_stale is a COALESCE and is never NULL"),
        has_materialized_at: has_mat,
        has_latest_touch: has_touch,
        touch_equals_watermark: equal,
    })
}

/// **The cogmap-side witness for the delegation**, mirroring `a_touched_context_reports_stale`.
///
/// `cogmap_staleness` no longer computes anything: it delegates to `anchor_staleness('kb_cogmaps',
/// …)`, whose regions arm reads `(home_anchor_table, home_anchor_id)` where the incumbent read
/// `reg.cogmap_id` (`migrations/20260823000020_anchor_staleness.sql`). The migration argues the
/// answer is preserved because those two carry the same value for a cogmap region — "by backfill and
/// convention, not by constraint". This pins that the preserved answer is the STALE one.
///
/// **Why it fails if the regions arm stops seeing the row.** This fixture homes exactly one region
/// on the map and no edges at all, so the regions arm is the ONLY source of `latest_touch`. Lose it
/// — by re-keying the arm, by a `NOT is_folded` predicate creeping in, or by a fixture that leaves
/// the anchor pair NULL — and the failure is not an error: `latest_touch` comes back NULL,
/// `latest_touch > materialized_at` is therefore NULL, and the `COALESCE`
/// (`20260624000002:549`, carried over unchanged) falls through to `materialized_at IS NULL`, which
/// is **false** for a map that has materialized once. So the post-touch `assert!(stale)` flips red,
/// and `assert!(has_touch)` flips red independently of it — a map that has materialized and never
/// been touched reports `is_stale = false` under BOTH the working and the broken arm, which is why
/// this test touches before asserting rather than only materializing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_touched_cogmap_reports_stale_through_the_delegating_wrapper(pool: PgPool) {
    let (profile, _context) =
        common::fixtures::create_test_profile_with_context(&pool, "mapclock@example.com").await;
    let (cogmap, region) =
        insert_materialized_cogmap_with_region(&pool, profile, "clock-map").await;

    let s = cogmap_staleness_row(&pool, cogmap, profile)
        .await
        .expect("a readable map that exists yields exactly one cogmap_staleness row");
    assert!(
        s.has_materialized_at,
        "the map was inserted with its watermark stamped"
    );
    assert!(
        s.has_latest_touch,
        "latest_touch is NULL only if the regions arm cannot see this map's region — i.e. the arm \
         and the row disagree about the key, which is the unconstrained equality the delegation \
         rests on"
    );
    assert!(
        !s.is_stale,
        "nothing has touched the map since its watermark, so it is fresh"
    );

    touch_region_with_a_later_event(&pool, profile, region).await;

    let s = cogmap_staleness_row(&pool, cogmap, profile)
        .await
        .expect("the gate does not move when the clock does");
    assert!(
        s.has_materialized_at && s.has_latest_touch,
        "both clocks are still readable"
    );
    assert!(
        s.is_stale,
        "a cogmap touched after materializing is stale — if this is false, the delegation lost the \
         region and every such map is permanently, silently fresh"
    );

    // The equivalence itself, stated as a differential rather than inferred: the wrapper's answer
    // must equal what the pre-delegation body computed, whose regions arm was keyed on
    // `reg.cogmap_id` (`20260624000002:538-541`). Both arms are reproduced here over the same row,
    // so a divergence localises to the key and nothing else.
    //
    // **The inline `old` arm is UNGATED, and that is what this fixture is arranged for.** Every row
    // it can see — the one region, its one member — is visible to `profile`, so the gate added by
    // `20260825000010` removes nothing here and the two answers must still agree. This comparison is
    // therefore a key test, NOT a gate test: it says the delegation preserved the answer where the
    // gate has nothing to do. The gate's own differential is the ghost-region and ghost-endpoint
    // witnesses below, where these two arms deliberately DISAGREE.
    let agrees: Option<bool> = sqlx::query_scalar(
        "SELECT new.is_stale IS NOT DISTINCT FROM old.is_stale
               AND new.latest_touch IS NOT DISTINCT FROM old.latest_touch
           FROM cogmap_staleness($1, 'profile', $2) new,
                LATERAL (
                  SELECT mat.materialized_at, t.latest_touch,
                         COALESCE(t.latest_touch > mat.materialized_at,
                                  mat.materialized_at IS NULL) AS is_stale
                    FROM (SELECT ev.occurred_at AS materialized_at
                            FROM kb_cogmaps m
                            LEFT JOIN kb_events ev ON ev.id = m.shape_materialized_event_id
                           WHERE m.id = $1) mat,
                         (SELECT max(occurred_at) AS latest_touch FROM (
                            SELECT ev.occurred_at FROM kb_cogmap_regions reg
                              JOIN kb_events ev ON ev.id = reg.last_event_id
                             WHERE reg.cogmap_id = $1
                            UNION ALL
                            SELECT ev.occurred_at FROM kb_edges e
                              JOIN kb_events ev ON ev.id = e.last_event_id
                             WHERE e.home_anchor_table = 'kb_cogmaps'
                               AND e.home_anchor_id = $1) x) t
                ) old",
    )
    .bind(cogmap)
    .bind(profile)
    .fetch_one(&pool)
    .await
    .expect("both arms answer over the same map");

    assert_eq!(
        agrees,
        Some(true),
        "the anchor-pair arm and the cogmap_id arm must agree for a region carrying both keys — \
         disagreement means the delegation changed the answer for cogmaps"
    );
}

// ---------------------------------------------------------------------------------------------
// THE GATE (`migrations/20260825000010_staleness_member_gate.sql`) — the clock stops reporting on
// rows the caller cannot read, and gains a context-side composer.
//
// **Why the tests above cannot serve as the witnesses for it.** Every one of them reads as the
// OWNER of a fully-visible fixture, and an owner sees all their own regions — so they answer
// identically before and after the gate. The design says so in as many words (§6: "a test that
// merely re-runs the current function proves reproducibility, not correctness"). The reachable
// bite is SOFT-DELETE and only soft-delete: `20260823000010:157-173` establishes that a readable
// anchor's regions are built from that anchor's own homes and `resources_visible_to` admits every
// resource homed in a readable anchor, so "a region holding members another tenant hid from you"
// is not a row the writer can produce — but a GHOST region, whose members were soft-deleted after
// materialize, is, and 40 of 40 dead-but-homed resources on prod were still region members.
//
// The incumbent body these bite against is quoted from the live catalog rather than paraphrased
// (`\sf anchor_staleness` on the pre-migration database):
//
//     touch AS (
//         SELECT max(occurred_at) AS latest_touch FROM (
//             SELECT ev.occurred_at FROM kb_cogmap_regions reg
//               JOIN kb_events ev ON ev.id = reg.last_event_id
//              WHERE reg.home_anchor_table = p_anchor_table
//                AND reg.home_anchor_id    = p_anchor_id
//             UNION ALL
//             SELECT ev.occurred_at FROM kb_edges e
//               JOIN kb_events ev ON ev.id = e.last_event_id
//              WHERE e.home_anchor_table = p_anchor_table
//                AND e.home_anchor_id    = p_anchor_id
//         ) t
//     )
//
// No readability predicate on either arm. That is what the next two tests are built to fail.
// ---------------------------------------------------------------------------------------------

/// **THE REGIONS-ARM BITE.** A region whose every member has been soft-deleted is a region the
/// caller can see nothing in, so its clock is not their clock — the same rule both enumeration
/// doors onto this anchor already applied (`anchor_shape` `20260823000010:87-88`,
/// `anchor_region_metrics` `20260713000050:262-268`), now applied to the staleness read.
///
/// **What the OLD function answers for THIS EXACT FIXTURE, which is why this test bites.** The
/// incumbent's regions arm (quoted in the block comment above) selects every region whose
/// `home_anchor_table`/`home_anchor_id` match, with no member predicate. This ghost region matches
/// the anchor pair, so `max(occurred_at)` over the arm is the `now() + interval '1 day'` event that
/// step 3 pointed `last_event_id` at. Therefore, under the incumbent:
///
///   * `latest_touch` is that later stamp — **NOT NULL**, so `assert!(!s.has_latest_touch)` fails;
///   * `latest_touch > materialized_at` is **true**, so `assert!(!s.is_stale)` fails.
///
/// Two independent assertions, both red. Under the gated function the region is excluded, the
/// context has no edges, so `latest_touch` is NULL, `NULL > materialized_at` is NULL, and the
/// `COALESCE` falls to `materialized_at IS NULL` — false, because step 1 materialized. The whole
/// point of step 1 is to make that fallback answer `false`: without it the assertion would be
/// satisfied by the never-clustered limb and prove nothing.
///
/// **Which conjunct this isolates: the MEMBER RULE, and only it.** The reader here is the context's
/// OWNER, so the gate's anchor disjunction (`anchor_readable_by_profile`) is satisfied — a build
/// carrying the anchor gate but no member rule returns a row with the ghost's touch in it and fails
/// this test. It says nothing about the anchor half; that is
/// `a_context_the_caller_cannot_read_yields_no_staleness_row`'s job.
///
/// **What it deliberately does NOT prove**, so nobody reads more into the `latest_touch IS NULL`
/// assertion than it carries: a build that deleted the regions arm outright would also pass. That
/// the arm still works is pinned by `a_touched_context_reports_stale` above, over a live member.
/// The pair is the witness; neither half is alone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_ghost_region_does_not_move_the_staleness_clock(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "ghost-region@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "the ghost").await;
    let member = insert_resource(&pool, context, owner, "a member, until it wasn't").await;
    add_member(&pool, region, member, 0.9).await;

    // 1. Materialize: sets the watermark, and makes `is_stale = false` mean FRESH rather than
    //    "never clustered" (the other limb of the same COALESCE).
    mark_context_materialized(&pool, context).await;

    // 2. Soft-delete every member. `resources_visible_to` ends in `AND r.is_active`, so the member
    //    is now invisible on every axis — including to this owner, who could see it a moment ago.
    //    The region itself is untouched: it still exists and still points at this context.
    soft_delete_resource(&pool, member).await;

    // 3. Advance the ghost's clock past the watermark. This context has no edges at all, so the
    //    ghost region is the ONLY thing under the anchor that has moved, and any `latest_touch` the
    //    read produces can only have come from it.
    touch_region_with_a_later_event(&pool, owner, region).await;

    // 4. Read as the OWNER — no second principal, which is what makes the fixture sharp rather than
    //    weak: the person who owns the anchor must stop seeing the clock move.
    let s = context_staleness(&pool, context, owner).await;

    assert!(
        s.has_materialized_at,
        "step 1 stamped the watermark — without it `is_stale = false` below would be the \
         never-clustered limb answering, not the freshness limb: {s:?}"
    );
    assert!(
        !s.has_latest_touch,
        "a region the caller can see NOTHING in must not reach the clock at all. A non-NULL \
         latest_touch here is the ungated regions arm counting the ghost's touch: {s:?}"
    );
    assert!(
        !s.is_stale,
        "...and therefore the anchor reads fresh. `is_stale = true` here is the disclosure this \
         migration closes: it reports that something moved under an anchor whose only movement was \
         in a region the shape read refuses to admit exists: {s:?}"
    );
}

/// **THE EDGES-ARM BITE.** An edge with an unreadable endpoint does not move this clock. The edges
/// arm has no member concept at all — it gates on `endpoint_readable_by_profile`
/// (`20260624000002:292`) on BOTH endpoints, which is the AUTHORIZATION half of `edges_visible_to`
/// taken on its own so the CURRENCY half (`NOT is_folded`) is not imported with it
/// (`20260825000010:221-231`).
///
/// **What the OLD function answers for THIS EXACT FIXTURE.** The incumbent's edges arm carried NO
/// predicate whatsoever — not even the endpoint check — so it takes `max(occurred_at)` over both
/// edges homed here. The ghost edge's `last_event_id` is the `now() + interval '1 day'` event, so:
///
///   * `latest_touch` is that later stamp, so `assert!(s.touch_equals_watermark)` fails;
///   * `latest_touch > materialized_at` is true, so `assert!(!s.is_stale)` fails.
///
/// Under the gated function the ghost edge is dropped (its source endpoint is soft-deleted, so
/// `endpoint_readable_by_profile` → `resources_visible_to` excludes it), and the only edge left is
/// the readable control, born at the watermark: `latest_touch = materialized_at`, `>` is false.
///
/// **Why the readable CONTROL edge is in the fixture.** Without it, "the ghost was gated out" and
/// "the edges arm was deleted" produce the same `latest_touch IS NULL`. With it, the arm must still
/// be alive AND still be selective, and `touch_equals_watermark` is what says so. This is the
/// discriminator the regions-arm witness above cannot have without giving up its `latest_touch IS
/// NULL` assertion, so it is stated here instead.
///
/// **Which conjunct this isolates: the EDGES-ARM ENDPOINT RULE, and only it.** The reader is the
/// owner, so the anchor disjunction passes; the context holds NO regions, so the member rule on the
/// regions arm has nothing to act on and cannot be what changes the answer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_ghost_endpoint_does_not_move_the_staleness_clock(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "ghost-edge@example.com").await;
    // NO regions on this context, deliberately: the edges arm must be the only contributor, or an
    // assertion about `latest_touch` could not be localised to it.
    mark_context_materialized(&pool, context).await;

    let live_a = insert_resource(&pool, context, owner, "one live endpoint").await;
    let live_b = insert_resource(&pool, context, owner, "the other live endpoint").await;
    let doomed = insert_resource(&pool, context, owner, "an endpoint, until it wasn't").await;

    // The CONTROL: both endpoints readable, born at the watermark, never moved.
    insert_context_edge(&pool, context, live_a, live_b).await;

    // The GHOST: one endpoint about to be soft-deleted, and the only thing that moves afterwards.
    let ghost_edge = insert_context_edge(&pool, context, doomed, live_b).await;
    soft_delete_resource(&pool, doomed).await;
    touch_edge_with_a_later_event(&pool, owner, ghost_edge).await;

    let s = context_staleness(&pool, context, owner).await;

    assert!(
        s.has_materialized_at,
        "the watermark is stamped, so `is_stale = false` below is the freshness limb: {s:?}"
    );
    assert!(
        s.has_latest_touch,
        "the readable control edge still reaches the clock — if this is NULL the edges arm was not \
         gated but REMOVED, which is a different defect and not the one under test: {s:?}"
    );
    assert!(
        s.touch_equals_watermark,
        "...and what it contributes is the watermark itself. The ghost edge's later stamp must not \
         be in the max(): if latest_touch has moved past materialized_at, the unreadable endpoint \
         was counted: {s:?}"
    );
    assert!(
        !s.is_stale,
        "an edge whose endpoint the caller cannot read does not make their anchor stale: {s:?}"
    );
}

/// **THE ANCHOR-GATE HALF (§3).** A caller who cannot read the anchor gets ZERO ROWS — not an
/// error, and not a row of NULLs.
///
/// **The assertion is deliberately `is_none()` and not `is_stale == false`, and that is the whole
/// content of this test.** Gating only the member/endpoint half would leave both arms contributing
/// nothing to a denied caller, `latest_touch` NULL, and the `COALESCE` collapsing to
/// `materialized_at IS NULL` — while the `mat` CTE reads the anchor's watermark UNCONDITIONALLY. So
/// a member-gated-but-anchor-ungated build hands a stranger ONE ROW carrying a real anchor's
/// `materialized_at` and an `is_stale` reporting whether it has ever been clustered. A test that
/// asserted `is_stale == false` would pass against exactly that build. Row COUNT is the only
/// assertion that separates them.
///
/// **Which conjunct this isolates: the ANCHOR DISJUNCTION, and only it.** The fixture's region holds
/// a live, visible-to-its-owner member, so the member rule admits it; the sole reason the stranger
/// gets nothing is `anchor_readable_by_profile`.
///
/// The owner read at the end is a POSITIVE CONTROL, not decoration: without it, `is_none()` could be
/// satisfied by a fixture that yields no row for anyone (an absent anchor, an unmaterialized one, a
/// broken insert), and the test would pass while proving nothing about the gate. The same fixture
/// must return a row to someone.
///
/// Both names are checked because `context_analytics` carries no gate of its own — it inherits the
/// core's (`20260825000010:327-330`) — so "the wrapper leaks what the core denies" is a real
/// regression shape and cannot be inferred from the core's own behaviour.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_context_the_caller_cannot_read_yields_no_staleness_row(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "gated@example.com").await;
    let stranger = common::fixtures::create_test_profile(&pool, "stranger@example.com").await;

    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "a live region").await;
    let member = insert_resource(&pool, context, owner, "a live member").await;
    add_member(&pool, region, member, 0.9).await;
    mark_context_materialized(&pool, context).await;
    // Touched after materializing: the anchor is genuinely STALE, so the row being withheld carries
    // a fact — not just a watermark, but a watermark plus "and something has moved since".
    touch_region_with_a_later_event(&pool, owner, region).await;

    let denied_core = anchor_staleness_row(&pool, "kb_contexts", context, stranger).await;
    assert!(
        denied_core.is_none(),
        "a caller who cannot read the anchor gets ZERO ROWS — deny and absence are the same answer, \
         so the row itself would be an existence-and-clock oracle: {denied_core:?}"
    );

    let denied_composer = context_analytics_row(&pool, context, stranger).await;
    assert!(
        denied_composer.is_none(),
        "...and the composer inherits that gate rather than carrying one of its own, so it must \
         deny identically: {denied_composer:?}"
    );

    // POSITIVE CONTROL: the same anchor, read by someone who may.
    let allowed = context_staleness(&pool, context, owner).await;
    assert!(
        allowed.is_stale && allowed.has_materialized_at,
        "the fixture really is a materialized, touched, readable anchor — so the two `is_none()` \
         assertions above are about the GATE and not about an anchor that answers to nobody: \
         {allowed:?}"
    );
}

/// One `context_analytics` row — the new context-side composer — or `None` on its zero-row deny.
/// Same shape and same reasoning as `anchor_staleness_row`; it exists separately so the composer is
/// exercised BY ITS OWN NAME, which is the surface the API, MCP and CLI wiring of Beat B will call.
async fn context_analytics_row(pool: &PgPool, context: Uuid, profile: Uuid) -> Option<Staleness> {
    let row: Option<(Option<bool>, bool, bool, bool)> = sqlx::query_as(
        "SELECT s.is_stale, \
                s.materialized_at IS NOT NULL, \
                s.latest_touch IS NOT NULL, \
                COALESCE(s.latest_touch = s.materialized_at, false) \
           FROM context_analytics($1, 'profile', $2) s",
    )
    .bind(context)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .expect("context_analytics is never an error — deny and absence are both zero rows");

    row.map(|(stale, has_mat, has_touch, equal)| Staleness {
        is_stale: stale.expect("is_stale is a COALESCE and is never NULL"),
        has_materialized_at: has_mat,
        has_latest_touch: has_touch,
        touch_equals_watermark: equal,
    })
}

/// `context_analytics` over a readable, materialized context: exactly ONE row, and its three columns
/// are the core's three columns.
///
/// The equality against `anchor_staleness` is asserted rather than assumed because the composer is a
/// delegating wrapper with no body of its own (`20260825000010:332-337`) — a hand-copied second
/// implementation is the thing that would drift, and this is the assertion that would notice.
///
/// The touch at the end is what makes the columns LIVE rather than constant: a wrapper returning
/// three literal NULLs, or a `materialized_at` read straight off the context row, would satisfy
/// every assertion before it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_analytics_returns_one_row_of_the_staleness_triple(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "analytics@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "a live region").await;
    let member = insert_resource(&pool, context, owner, "a live member").await;
    add_member(&pool, region, member, 0.9).await;
    mark_context_materialized(&pool, context).await;

    let rows: Vec<i32> = sqlx::query_scalar("SELECT 1 FROM context_analytics($1, 'profile', $2)")
        .bind(context)
        .bind(owner)
        .fetch_all(&pool)
        .await
        .expect("context_analytics over a readable context is Ok");
    assert_eq!(
        rows.len(),
        1,
        "a readable, existing context yields exactly one analytics row: {rows:?}"
    );

    let fresh = context_analytics_row(&pool, context, owner)
        .await
        .expect("...and it decodes");
    assert!(
        fresh.has_materialized_at && fresh.has_latest_touch,
        "both clocks are readable through the composer: {fresh:?}"
    );
    assert!(
        !fresh.is_stale,
        "nothing has moved since the watermark: {fresh:?}"
    );

    // The delegation, stated as an equality rather than inferred from two separately-asserted
    // values: whatever the core says, the composer says, column for column.
    let agrees: Option<bool> = sqlx::query_scalar(
        "SELECT a.materialized_at IS NOT DISTINCT FROM s.materialized_at
               AND a.latest_touch IS NOT DISTINCT FROM s.latest_touch
               AND a.is_stale     IS NOT DISTINCT FROM s.is_stale
           FROM context_analytics($1, 'profile', $2) a,
                anchor_staleness('kb_contexts', $1, 'profile', $2) s",
    )
    .bind(context)
    .bind(owner)
    .fetch_one(&pool)
    .await
    .expect("both reads answer over the same context");
    assert_eq!(
        agrees,
        Some(true),
        "context_analytics must be the core's answer verbatim — it has no body of its own to \
         disagree with it"
    );

    touch_region_with_a_later_event(&pool, owner, region).await;
    let touched = context_analytics_row(&pool, context, owner)
        .await
        .expect("the composer still answers after a touch");
    assert!(
        touched.is_stale,
        "the composer's columns track the live clock — if this is false it is reporting something \
         static rather than delegating: {touched:?}"
    );
}

/// `context_analytics` returns THREE columns, not the five its cogmap peer returns (§4).
///
/// A context has no charter resource and no regulation set, so `telos_resource_id NULL` and
/// `regulation '[]'` would say *nothing found* about two things that cannot exist — the exact
/// failure `CONTEXT_HAS_NO_MAP_READOUT` was written to avoid. The return type IS that design
/// decision, so it is pinned from the live catalog rather than left to a reviewer's eye.
///
/// Read as an equality against `anchor_staleness`'s own result type rather than against a
/// hard-coded string, so the assertion survives a rename of the triple and fails only on a genuine
/// shape divergence.
///
/// This test does not bite against the incumbent in the way the two ghost witnesses do — before this
/// migration `context_analytics` did not exist at all, so there is no "old answer" for it to differ
/// from. It is a shape guard against a future "make it a peer of cogmap_analytics" edit.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn context_analytics_returns_three_columns_and_not_its_cogmap_peers_five(pool: PgPool) {
    let (context_shape, core_shape, peer_shape): (String, String, String) = sqlx::query_as(
        "SELECT (SELECT pg_get_function_result(oid) FROM pg_proc \
                  WHERE pronamespace = 'public'::regnamespace \
                    AND proname = 'context_analytics'), \
                (SELECT pg_get_function_result(oid) FROM pg_proc \
                  WHERE pronamespace = 'public'::regnamespace \
                    AND proname = 'anchor_staleness'), \
                (SELECT pg_get_function_result(oid) FROM pg_proc \
                  WHERE pronamespace = 'public'::regnamespace \
                    AND proname = 'cogmap_analytics')",
    )
    .fetch_one(&pool)
    .await
    .expect("all three functions exist under exactly one signature each");

    assert_eq!(
        context_shape, core_shape,
        "context_analytics returns the staleness triple its core returns, unchanged"
    );
    assert_ne!(
        context_shape, peer_shape,
        "...and NOT the five columns of cogmap_analytics: a context has no charter resource and no \
         regulation set, so those two would be null peer fields reporting `nothing found` about \
         something that cannot exist"
    );
}

/// **§4: the old ungated signatures are GONE, not standing beside the new ones as overloads.**
///
/// In Postgres, adding a parameter creates an overload rather than replacing a function. A
/// `CREATE OR REPLACE` at the longer argument list would have left `anchor_staleness(text, uuid)`
/// and `cogmap_staleness(uuid)` in the catalog — same name, same column names, same `boolean` type,
/// no gate — and every existing caller would keep resolving to them. Nothing errors, nothing goes
/// red, and the fix silently does not apply. That is misrouting, not drift.
///
/// The hazard is not hypothetical for this schema: `__temper_ungated_follow_from` exists under three
/// signatures at once, each a generation that was added rather than replaced
/// (`20260825000010:115-117`).
///
/// So this asserts the catalog holds EXACTLY the gated signature for each name. It is the only test
/// in this file that a correct-looking migration can fail while every behavioural test above still
/// passes — because the behavioural tests bind their arguments, and a bound four-argument call
/// resolves to the gated function whether or not the two-argument one is still there.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_old_ungated_staleness_signatures_are_dropped_not_overloaded(pool: PgPool) {
    let signatures: Vec<String> = sqlx::query_scalar(
        "SELECT p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')' \
           FROM pg_proc p \
          WHERE p.pronamespace = 'public'::regnamespace \
            AND p.proname IN ('anchor_staleness', 'cogmap_staleness', 'context_analytics') \
          ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("read the live function catalog");

    assert_eq!(
        signatures,
        vec![
            "anchor_staleness(p_anchor_table text, p_anchor_id uuid, p_principal_kind text, \
             p_principal_id uuid)"
                .to_string(),
            "cogmap_staleness(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)"
                .to_string(),
            "context_analytics(p_context uuid, p_principal_kind text, p_principal_id uuid)"
                .to_string(),
        ],
        "one signature per name, each taking a principal. An extra row here is the ungated \
         incumbent still standing and still absorbing callers: {signatures:?}"
    );
}

// ── The fold arms, which must NOT be narrowed ────────────────────────────────
//
// These two do NOT bite against the incumbent — the incumbent is fold-inclusive too, so they pass
// against both. They are here for the opposite reason: they PROTECT an arm from being "tightened"
// later by someone reconciling it with `anchor_shape`, which does carry `NOT reg.is_folded`
// (`20260823000010:86`). The distinction that makes the asymmetry deliberate rather than an
// oversight, from `internal/agents/key-patterns.md` and restated at `20260825000010:62-71`:
// `is_active` and `resources_visible_to` are AUTHORIZATION predicates, `is_folded` is a CURRENCY
// one. This migration added authorization and touched currency nowhere.
//
// The failure a narrowing would cause is silent: a fold advances `last_event_id`, so dropping
// folded rows makes a STALE anchor read FRESH, with every value still a plausible timestamp and
// nothing to error on. The covering index was built for exactly this folded-inclusive scan
// (`20260708000008:10-15`, `idx_kb_edges_home_all`).

/// A FOLDED region still moves the staleness clock.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_folded_region_still_moves_the_staleness_clock(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "fold-region@example.com").await;
    let region = insert_context_region(&pool, context, 0.9, Some(0.5), "the folded region").await;
    // A LIVE, visible member: the member rule must pass, so that the only predicate this fixture
    // can be failed by is a fold predicate. Without it the test would go green for the wrong reason
    // under a narrowing — gated out by the member rule instead of admitted despite the fold.
    let member = insert_resource(&pool, context, owner, "a live member").await;
    add_member(&pool, region, member, 0.9).await;
    mark_context_materialized(&pool, context).await;

    sqlx::query("UPDATE kb_cogmap_regions SET is_folded = true WHERE id = $1")
        .bind(region)
        .execute(&pool)
        .await
        .expect("fold the region");
    touch_region_with_a_later_event(&pool, owner, region).await;

    let s = context_staleness(&pool, context, owner).await;
    assert!(
        s.has_latest_touch && s.is_stale,
        "a fold IS a touch — it advances last_event_id. If the regions arm ever grows a \
         `NOT reg.is_folded` predicate to match anchor_shape's, this anchor reads FRESH while being \
         stale, and nothing else in the suite would notice: {s:?}"
    );
}

/// A FOLDED edge still moves the staleness clock.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_folded_edge_still_moves_the_staleness_clock(pool: PgPool) {
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&pool, "fold-edge@example.com").await;
    // No regions: the edges arm is the only contributor, so the assertion localises to it.
    mark_context_materialized(&pool, context).await;

    // Both endpoints live and visible, so the endpoint rule passes and a fold predicate is the only
    // thing that could drop this edge.
    let live_a = insert_resource(&pool, context, owner, "one live endpoint").await;
    let live_b = insert_resource(&pool, context, owner, "the other live endpoint").await;
    let edge = insert_context_edge(&pool, context, live_a, live_b).await;

    sqlx::query("UPDATE kb_edges SET is_folded = true WHERE id = $1")
        .bind(edge)
        .execute(&pool)
        .await
        .expect("fold the edge");
    touch_edge_with_a_later_event(&pool, owner, edge).await;

    let s = context_staleness(&pool, context, owner).await;
    assert!(
        s.has_latest_touch && s.is_stale,
        "the edges arm is folded-inclusive on purpose, and calling `edges_visible_to` wholesale \
         would have imported the `NOT e.is_folded` at 20260712000010:297 through the back door — \
         which is why the migration composes `endpoint_readable_by_profile` instead: {s:?}"
    );
}
