#![cfg(feature = "test-db")]
//! The `resource_count` on a context read is counted over what the CALLER may read.
//!
//! A returned count is a disclosure about every row that can move it. `context_visible_to` is the
//! ANCHOR gate — it decides whether the caller gets an answer at all — and says nothing about the
//! contents of the context. `kb_resource_homes` carries neither `is_active` nor a reader column
//! (`20260624000001:276-284`), so a count taken straight off the home rows reports resources that
//! have been soft-deleted and, in principle, resources the caller cannot read. The correct counted
//! set was already written down for this same field on this same anchor — `graph_home_contexts`
//! joins `resources_visible_to` AND the `kb_resources.is_active` floor (`20260712000010:327-331`) —
//! and this test pins `context_service::list_visible` to it, so the two doors onto one context
//! cannot disagree about which resources exist.
//!
//! **Why this bites.** Before the fix the list query read `COUNT(rh.resource_id)` over a bare
//! `LEFT JOIN kb_resource_homes` — neither of the two joins the fix adds was there. The fixture
//! below homes two resources in a context and soft-deletes one; the old query answers `2` (the home
//! row survives a soft delete — the projector's whole effect is the `is_active` flag) where the
//! enumeration a caller can actually reach answers `1`. For this test to pass against the old code
//! the count would have to have been restricted to live resources — but it does NOT follow that the
//! `kb_resources` join is what restricts it, and the next paragraph is why. The pre-delete
//! assertion is deliberately kept so a "fix" that simply returned a smaller number could not pass
//! either.
//!
//! **What this does NOT cover, 1: neither conjunct is isolated.** On a context anchor the two added
//! joins are today CO-EXTENSIVE — they compute the identical set. `resources_visible_to` ENDS with
//! its own soft-delete floor, `JOIN kb_resources r ON r.id = v.resource_id AND r.is_active`
//! (`20260807000010:223-225`), so it can never admit a dead resource; and in the other direction its
//! context arm admits every home in a context `contexts_readable_by` admits
//! (`20260807000010:199-205`), which is the same set `context_visible_to` gates the row on
//! (`20260712000010:147-151`), so every live home this subquery can reach is already inside it.
//! Delete `JOIN kb_resources … AND r.is_active` and this test still passes; delete
//! `JOIN resources_visible_to($1) v` and it still passes. So it pins ONE property — the count
//! excludes a soft-deleted resource — and attributes it to NEITHER predicate. The third assertion
//! below is this file's own proof of that: it enumerates through `resources_visible_to` ALONE, with
//! no `kb_resources` join, and gets `vec![live]`. `graph_home_contexts` carries the identical
//! doubling on this same field (`20260712000010:329-330`), so the fixed query is faithfully copying
//! a pattern that already ships — but that pattern's redundancy is nowhere else written down, so it
//! is written down here. Both joins are kept as mutual backstops and for symmetry with the sibling,
//! never as two independent filters.
//!
//! **What this does NOT cover, 2: the visibility half.** For the same reason, a LIVE resource the
//! caller cannot read but that is homed in a context they CAN read is not constructible on a
//! context anchor today. So no fixture in this repo can witness the reader predicate moving the
//! number until one of those two functions narrows. It is kept anyway: it is what makes the count
//! correct BECAUSE of the caller rather than by a coincidence between two functions that may yet
//! narrow.

mod common;

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_services::services::context_service;
use uuid::Uuid;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// Home a resource in `context_id`, owned/originated by `owner_id`. Returns the resource id.
async fn home_resource_in_context(
    pool: &PgPool,
    context_id: Uuid,
    owner_id: Uuid,
    title: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(title)
        .bind(format!("test://{id}"))
        .execute(pool)
        .await
        .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
            (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(context_id)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("home resource");
    id
}

/// Soft-delete a resource the way the `resource_deleted` projector does — the flag IS the whole
/// effect, and the `kb_resource_homes` row is deliberately left in place. Written as a bare UPDATE
/// so the test cannot pass because some service helper also removed the home.
async fn soft_delete_resource(pool: &PgPool, resource_id: Uuid) {
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(resource_id)
        .execute(pool)
        .await
        .expect("soft-delete resource");
}

/// The `resource_count` `list_visible` reports for `context_id`.
async fn listed_resource_count(pool: &PgPool, principal: ProfileId, context_id: Uuid) -> i64 {
    let rows = context_service::list_visible(pool, principal)
        .await
        .expect("list_visible succeeds for the owner");
    rows.iter()
        .find(|r| *r.id == context_id)
        .expect("the caller's own context is in their list")
        .resource_count
}

// ─── The witness ──────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_count_excludes_a_soft_deleted_resource(pool: PgPool) {
    let email = format!("rc-floor-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let principal = ProfileId::from(profile_id);

    let live = home_resource_in_context(&pool, context_id, profile_id, "kept").await;
    let doomed = home_resource_in_context(&pool, context_id, profile_id, "deleted").await;

    assert_eq!(
        listed_resource_count(&pool, principal, context_id).await,
        2,
        "precondition: both live resources are counted, so a count that under-reports for some \
         other reason cannot masquerade as the fix"
    );

    soft_delete_resource(&pool, doomed).await;

    assert_eq!(
        listed_resource_count(&pool, principal, context_id).await,
        1,
        "a soft-deleted resource leaves the count: the home row outlives the delete, so a count \
         taken off kb_resource_homes alone still says 2 and tells the caller about a resource no \
         enumeration they can reach will return"
    );

    // The number must equal what the caller can actually enumerate — one, and it is `live`.
    // Deliberately NO `kb_resources` join here: this is the module doc's own disproof of the
    // idea that `is_active` is what fixed the count. `resources_visible_to` alone already
    // drops the soft-deleted row, because it ends with that floor itself.
    let visible: Vec<Uuid> = sqlx::query_scalar(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN resources_visible_to($1) v ON v.resource_id = h.resource_id \
          WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = $2",
    )
    .bind(profile_id)
    .bind(context_id)
    .fetch_all(&pool)
    .await
    .expect("enumerate the caller's visible homes");
    assert_eq!(
        visible,
        vec![live],
        "the counted set must be the enumerable set, not merely the same cardinality"
    );
}
