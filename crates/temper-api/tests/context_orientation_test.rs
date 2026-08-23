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

/// Mint an event that occurred strictly AFTER the fixture's watermark, and point one region's
/// `last_event_id` at it — the "touched since it materialized" half of the witness below.
///
/// The timestamp is explicit (`now() + interval '1 day'`) rather than the column default:
/// `mark_context_materialized` stamps a migration-seeded event whose `occurred_at` is only
/// microseconds behind the test's own clock, and the assertion turns on
/// `latest_touch > materialized_at` being unambiguously true rather than on two stamps that tie.
async fn touch_region_with_a_later_event(pool: &PgPool, profile: Uuid, region: Uuid) {
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

    let event: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, occurred_at) \
         SELECT (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'), $1, \
                now() + interval '1 day' \
         RETURNING id",
    )
    .bind(entity)
    .fetch_one(pool)
    .await
    .expect("mint an event later than the watermark");

    sqlx::query("UPDATE kb_cogmap_regions SET last_event_id = $2 WHERE id = $1")
        .bind(region)
        .bind(event)
        .execute(pool)
        .await
        .expect("advance the region's clock");
}

/// One `anchor_staleness` row over a context, as `(is_stale, has_materialized_at, has_latest_touch)`.
///
/// Runtime `query_scalar`/`query_as`, not the macro: the function is brand new, and a
/// compile-time-checked call would demand a `.sqlx` cache entry that only `cargo sqlx prepare`
/// against a migrated database can produce. `is_stale` is read as `Option<bool>` because
/// `RETURNS TABLE` declares it nullable even though the `COALESCE` never yields NULL — a NULL here
/// would itself be a finding, so it is unwrapped with a message rather than decoded into `bool`.
async fn context_staleness(pool: &PgPool, context: Uuid) -> (bool, bool, bool) {
    let (stale, has_mat, has_touch): (Option<bool>, bool, bool) = sqlx::query_as(
        "SELECT s.is_stale, s.materialized_at IS NOT NULL, s.latest_touch IS NOT NULL \
           FROM anchor_staleness('kb_contexts', $1) s",
    )
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("anchor_staleness yields exactly one row for a context that exists");

    (
        stale.expect("is_stale is a COALESCE and is never NULL"),
        has_mat,
        has_touch,
    )
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
    // `insert_context_region` stamps the region's clock with the earliest seeded event and
    // `mark_context_materialized` stamps the watermark with the same one, so the context starts
    // materialized-and-untouched: the two timestamps are equal and `>` is false.
    mark_context_materialized(&pool, context).await;

    let (stale, has_mat, has_touch) = context_staleness(&pool, context).await;
    assert!(has_mat, "the context was just materialized");
    assert!(
        has_touch,
        "latest_touch is NULL only if the regions arm cannot see this context's regions — i.e. it \
         is still keyed on the vestigial cogmap_id instead of the anchor pair"
    );
    assert!(
        !stale,
        "nothing has touched the context since its watermark, so it is fresh"
    );

    touch_region_with_a_later_event(&pool, profile, region).await;

    let (stale, has_mat, has_touch) = context_staleness(&pool, context).await;
    assert!(has_mat && has_touch, "both clocks are still readable");
    assert!(
        stale,
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
    let (_profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, "unclustered@example.com").await;
    insert_context_region(&pool, context, 0.5, None, "the region").await;

    let (stale, has_mat, _) = context_staleness(&pool, context).await;
    assert!(!has_mat, "the fixture context carries no watermark yet");
    assert!(stale, "never materialized reads as stale, not as fresh");
}

/// A context that does not exist yields ZERO rows, not a row of NULLs — the behaviour
/// `cogmap_analytics` already depends on ("cogmap_staleness yields exactly one row",
/// `migrations/20260628000001_cogmap_analytics_read_functions.sql:25-26`), which is what makes its
/// gate in `WHERE` deny to an empty result rather than to a spurious row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_absent_anchor_yields_no_row(pool: PgPool) {
    let rows: Vec<i32> = sqlx::query_scalar("SELECT 1 FROM anchor_staleness('kb_contexts', $1)")
        .bind(Uuid::now_v7())
        .fetch_all(&pool)
        .await
        .expect("an absent anchor is not an error");

    assert!(
        rows.is_empty(),
        "an anchor that does not exist has no clock to report: {rows:?}"
    );
}
