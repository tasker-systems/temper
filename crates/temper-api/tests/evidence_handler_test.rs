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

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn readable_finding_returns_full_shape(pool: PgPool) {
    // A profile-owned context + goal: the goal is readable by its owner
    // (`resources_readable_by('profile', owner)` = `resources_visible_to(owner)`, which admits
    // profile-owned homes). `n = 0` → the goal only, no child tasks.
    let (owner, _ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;

    let shape = resource_evidence(&pool, owner, goal)
        .await
        .expect("owner reads its own finding's standing");

    // The shape describes the requested finding and carries the band chip WITH it.
    assert_eq!(shape.finding_id, ResourceId::from(goal), "shape: {shape:?}");
    // A finding with no emitted evidence has zeroed components and the lowest band.
    assert_eq!(shape.challenge_count, 0, "no challenges yet: {shape:?}");
    assert_eq!(
        shape.band, "provisional",
        "a no-evidence finding bands provisional: {shape:?}"
    );
    // Every numeric component is present (non-NaN) — `band` is a lossy summary OVER these.
    assert!(shape.indep_breadth.is_finite(), "shape: {shape:?}");
    assert!(shape.adversarial_survival.is_finite(), "shape: {shape:?}");
    assert!(shape.contradiction_balance.is_finite(), "shape: {shape:?}");
    assert!(shape.freshness.is_finite(), "shape: {shape:?}");
    assert!(shape.r_parent.is_finite(), "shape: {shape:?}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unreadable_finding_is_not_found(pool: PgPool) {
    // Seed a finding owned by one profile, then read it as a stranger who cannot see it.
    let (_owner, _ctx, goal) = common::seed_context_with_goal_and_tasks(&pool, 0).await;
    let stranger = common::seed_profile(&pool, "stranger").await;

    // An existing but unreadable finding: the in-SQL gate yields zero rows → None → NotFound.
    let existing = resource_evidence(&pool, stranger, goal).await;
    assert!(
        matches!(existing, Err(ApiError::NotFound)),
        "the in-SQL gate hides an unreadable finding as NotFound: {existing:?}"
    );

    // An absent finding is likewise NotFound (leak-safe: denied and absent are indistinguishable).
    let absent = resource_evidence(&pool, stranger, Uuid::now_v7()).await;
    assert!(
        matches!(absent, Err(ApiError::NotFound)),
        "an absent finding is NotFound: {absent:?}"
    );
}
