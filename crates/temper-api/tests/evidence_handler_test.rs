#![cfg(feature = "test-db")]
//! `evidential_standing_service::resource_evidence` — the api-side service-direct wrapper over the
//! `readback::resource_standing` binding (Set 3 Phase C, Task 8). Proves the readable path returns
//! Ok with the full 8-field shape (the `band` chip carried WITH the shape, never in place of it),
//! and that an unreadable/absent finding yields `ApiError::NotFound` — the access gate lives INSIDE
//! the SQL (`resource_standing_shape`'s `gated` CTE over `resources_readable_by`), surfaced as
//! `None` → 404. Same approach as `cogmap_shape_handler_test` / `invocation_handler_test`; full HTTP
//! routing is covered by a later e2e task (Task 9).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ResourceId;
use temper_services::error::ApiError;
use temper_services::services::evidential_standing_service::resource_evidence;

mod common;

/// Give `finding` one live resource-kind citation: a body block, a cited resource, and the
/// `kb_block_provenance` row that joins them — the exact three rows `resource_live_citations`
/// walks. Returns the cited source's id.
///
/// Raw inserts, mirroring the sibling fixture in `citation_audit_handler_test.rs:37-100` (this
/// crate's test files each carry their own fixtures; `common::seed_resource` deliberately seeds no
/// content block). The contributing event is borrowed from the scaffold — nothing under test reads
/// it, and `resource_citation_magnitude` never looks at `contributed_by_event_id`.
async fn cite_one_source_on(pool: &PgPool, finding: Uuid) -> Uuid {
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("the context scaffold seeded an event");
    let block = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, $2, 0, $3, $3)",
    )
    .bind(block)
    .bind(finding)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert block");

    let source = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(source)
        .bind("Cited Source")
        .bind(format!("test://{source}"))
        .execute(pool)
        .await
        .expect("insert cited source");
    sqlx::query(
        "INSERT INTO kb_block_provenance \
             (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
         VALUES ($1, 'resource', $2, $3, 0)",
    )
    .bind(block)
    .bind(source)
    .bind(event)
    .execute(pool)
    .await
    .expect("cite the source on the block");

    source
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn readable_finding_returns_full_shape(pool: PgPool) {
    // A profile-owned context + goal: the goal is readable by its owner
    // (`resources_readable_by('profile', owner)` = `resources_visible_to(owner)`, which admits
    // profile-owned homes). `n = 0` → the goal only, no child tasks.
    let (owner, _ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    // ONE live resource-kind citation. Without it every numeric component of this shape is zero,
    // and the assertions below hold just as well against producers that return a constant — which
    // is what they used to do: `citation_magnitude` was not asserted at all, and `audit_coverage`,
    // `band` and an `is_finite()` on `citation_quality` all pass against a hardcoded zero shape.
    // A seeded citation makes `citation_magnitude == 1` a value only a real computation produces.
    cite_one_source_on(&pool, goal).await;

    let shape = resource_evidence(&pool, owner, goal)
        .await
        .expect("owner reads its own finding's standing");

    // The shape describes the requested finding and carries the band chip WITH it.
    assert_eq!(shape.finding_id, ResourceId::from(goal), "shape: {shape:?}");
    assert_eq!(
        shape.citation_magnitude, 1,
        "the one seeded live resource-kind citation is counted — a constant producer reds here: \
         {shape:?}"
    );
    assert_eq!(
        shape.r_parent, 1.0,
        "and the same provenance row is counted by r_parent, which is a DIFFERENT producer over \
         the same rows: {shape:?}"
    );
    // Cited but unaudited: the axes are independent, and this is the state the auditor's queue
    // exists to clear.
    assert_eq!(
        shape.audit_coverage, 0,
        "nobody has audited that citation yet: {shape:?}"
    );
    assert_eq!(
        shape.citation_quality, 0.0,
        "an unaudited finding makes no quality claim — neutral 0.0, not a negative prior: {shape:?}"
    );
    assert_eq!(
        shape.band, "provisional",
        "cited, unaudited, uncontradicted: the floor: {shape:?}"
    );
    // The remaining numeric components are present (non-NaN) — `band` is a lossy summary OVER these.
    assert!(shape.contradiction_balance.is_finite(), "shape: {shape:?}");
    assert!(shape.freshness.is_finite(), "shape: {shape:?}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_finding_is_not_found(pool: PgPool) {
    // Seed a finding owned by one profile, then read it as a stranger who cannot see it.
    let (_owner, _ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let stranger = common::seed_profile(&pool, "stranger").await;

    // An existing but unreadable finding: the in-SQL gate yields zero rows → None → NotFound.
    let existing = resource_evidence(&pool, stranger, goal).await;
    assert!(
        matches!(existing, Err(ApiError::NotFound(_))),
        "the in-SQL gate hides an unreadable finding as NotFound: {existing:?}"
    );

    // An absent finding is likewise NotFound (leak-safe: denied and absent are indistinguishable).
    let absent = resource_evidence(&pool, stranger, Uuid::now_v7()).await;
    assert!(
        matches!(absent, Err(ApiError::NotFound(_))),
        "an absent finding is NotFound: {absent:?}"
    );
}
