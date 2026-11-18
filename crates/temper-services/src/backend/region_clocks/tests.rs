use super::*;
use sqlx::PgPool;
use uuid::Uuid;

use crate::test_support::{context, entity, profile};

/// A context with an owner and an emitter entity. Empty of resources on purpose: the formation
/// clock's gate is an event COUNT against the watermark, and an empty context materializes to zero
/// regions without needing embeddings — so this exercises the gate, not the producer.
struct Seeded {
    context: Uuid,
    entity: Uuid,
}

async fn seed(pool: &PgPool) -> Seeded {
    let owner = profile::insert(pool, "owner").await;
    let context = context::insert_personal(pool, owner, "ctx", "Ctx").await;
    let entity = entity::insert(pool, owner, "e").await;
    Seeded { context, entity }
}

/// A synthetic formation event anchored to the CONTEXT — what a resource write leaves behind, and
/// what the expensive clock counts.
async fn add_context_event(pool: &PgPool, entity: Uuid, type_name: &str, context: Uuid) {
    sqlx::query(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         VALUES ((SELECT id FROM kb_event_types WHERE name = $1), $2, 'kb_contexts', $3)",
    )
    .bind(type_name)
    .bind(entity)
    .bind(context)
    .execute(pool)
    .await
    .unwrap();
}

fn anchor_of(context: Uuid) -> HomeAnchor {
    HomeAnchor::Context(temper_core::types::ids::ContextId::from(context))
}

/// **The acceptance criterion: region production fires on CONTEXT writes.**
///
/// Worth stating plainly, because before this the trigger did not exist at all — and could not
/// have, for a context: `materialize_on_threshold` is `CogmapId`-typed and is only ever reached
/// from an explicit endpoint. Nothing on any resource-write path called it.
#[sqlx::test(migrations = "../../migrations")]
async fn the_formation_clock_fires_on_a_context_once_the_threshold_clears(pool: PgPool) {
    let s = seed(&pool).await;
    let anchor = anchor_of(s.context);

    for _ in 0..3 {
        add_context_event(&pool, s.entity, "resource_created", s.context).await;
    }
    let first = tick(&pool, anchor, s.entity.into(), Some(3)).await.unwrap();
    assert!(
        first.materialized,
        "3 formation events >= threshold 3 — the expensive clock must fire on a CONTEXT anchor"
    );

    // It advanced the watermark, so the same events no longer count: an immediate second tick is a
    // no-op. Without this the gate would re-cluster on every write forever.
    let again = tick(&pool, anchor, s.entity.into(), Some(3)).await.unwrap();
    assert!(
        !again.materialized,
        "the materialize advanced shape_materialized_event_id — those events are now behind the \
         watermark and must not re-trigger"
    );
}

/// Below threshold the tick is a cheap no-op: one drift read and one count(*), and it must not
/// touch the substrate. This is what makes it affordable to fire inline on EVERY resource write.
#[sqlx::test(migrations = "../../migrations")]
async fn below_threshold_and_with_no_telos_the_tick_does_nothing(pool: PgPool) {
    let s = seed(&pool).await;
    add_context_event(&pool, s.entity, "resource_created", s.context).await;

    let ticked = tick(&pool, anchor_of(s.context), s.entity.into(), Some(5))
        .await
        .unwrap();
    assert!(!ticked.materialized, "1 < 5");
    assert!(
        !ticked.salience_refreshed,
        "an un-materialized context has no telos snapshot, so drift is NULL — and a NULL must \
         DECLINE to fire the cheap clock, never fire it spuriously"
    );
}

/// A resource with no home row has no anchor whose regions it could affect — not an error, just
/// nothing to tick. (`update_resource` resolves the anchor this way, so a null result must be a
/// quiet skip and not a 500 on a legitimate write.)
#[sqlx::test(migrations = "../../migrations")]
async fn a_homeless_resource_has_no_clocks_to_tick(pool: PgPool) {
    let orphan: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('orphan','') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(home_of(&pool, orphan).await.unwrap(), None);
}
