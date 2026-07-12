//! T6 — the two clocks (spec §3.5). Fired inline on every resource write.
//!
//! Formation is expensive and depends on membership inputs. Salience is a handful of cosines and
//! depends on the telos. **In a cogmap they move together**, so running the readouts only inside
//! `materialize` was fine. **In a context they come apart** — goals open and close without any
//! region's membership changing. The shape is identical; what matters has moved. Gating the cheap
//! thing behind the expensive thing would mean a goal closing has no visible effect until ~5 unrelated
//! writes trip the formation threshold.
//!
//! The separation is structural, not hopeful. Formation reads members, edges, facets
//! (`property_key='facet'` only) and embeddings. Liveness reads `temper-stage` property rows and
//! `advances` edges. **The two input sets are disjoint** — so closing a task rewrites a `temper-stage`
//! row, which is in the second set and not the first, and membership *cannot* move while the telos
//! *must*. (T5 pinned it in CI:
//! `context_telos_salience.rs::closing_a_goal_moves_salience_without_changing_region_membership`.)
//!
//! ```text
//! on write to anchor A:
//!   1. telos drift    d = 1 − cos(telos_now(A), A.telos_centroid)      -- one cosine
//!                     if d > ε:  refresh_salience(A)                   -- cheap: NO clustering
//!                                A.telos_centroid := telos_now(A)
//!
//!   2. formation      n = formation_touched_count_since(A, watermark)  -- one count(*)
//!                     if n ≥ threshold:  incremental_materialize(A)    -- expensive
//! ```
//!
//! **No cron, no steward agent** — the whole point of a context is real-time capture. Note the task
//! text said this gate already existed on `materialize_on_threshold`; it did not. That function is
//! `CogmapId`-typed and is only ever reached from an explicit `POST …/materialize` endpoint and the
//! MCP tool. Nothing on any resource-write path called it, for contexts *or* cogmaps. This module is
//! that missing trigger.
//!
//! ## Why this never fails a write
//!
//! Region production is a **projection** over committed substrate — it derives what is already true.
//! A user's resource create/update has already committed by the time these clocks tick, and if a
//! clock fails the resource is still correct, still readable, still searchable; only its region
//! geometry is briefly stale, and the very next write re-drives both clocks from the same watermarks.
//! So a failure here is logged and swallowed, exactly as the embed-backfill enqueue beside it is.
//! Escalating it would trade a self-healing staleness for a user-visible 500.

use temper_core::types::home::HomeAnchor;
use temper_core::types::materialize::{default_lens_for, DEFAULT_MATERIALIZE_THRESHOLD};
use temper_substrate::ids::EntityId;
use temper_substrate::{replay, substrate, write};

/// What the two clocks did on one write — returned for tests and tracing, not for the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClockTick {
    /// The cheap clock fired: the telos had moved past the lens's epsilon.
    pub salience_refreshed: bool,
    /// The expensive clock fired: enough formation events had accumulated.
    pub materialized: bool,
}

/// Tick both clocks for an anchor after a resource write.
///
/// Errors are returned so tests can assert on them; the production caller
/// (`DbBackend::tick_region_clocks`) logs and swallows — see the module docs.
pub async fn tick(
    pool: &sqlx::PgPool,
    anchor: HomeAnchor,
    emitter: EntityId,
    threshold: Option<i64>,
) -> anyhow::Result<ClockTick> {
    let lens_name = default_lens_for(anchor);
    let (_, lens_id) = substrate::load_lens(pool, anchor, lens_name).await?;
    let mut tick = ClockTick::default();

    // ── clock 1: the telos. One cosine against the snapshot.
    //
    // Runs BEFORE the formation check and independently of it, rather than being skipped when
    // formation is about to fire. Formation only re-populates the readouts of components it actually
    // re-clusters (plus reused regions whose *content* moved) — so in a multi-component anchor, a
    // region in an untouched component would keep a stale telos term even though the anchor's purpose
    // moved. Refreshing all live regions here first closes that hole. When both clocks fire the
    // overlap is two set-based UPDATEs, which is nothing beside a re-cluster.
    let drift = write::telos_drift(pool, anchor, lens_id).await?;
    if drift.exceeds_epsilon {
        write::refresh_salience(pool, anchor, lens_name, emitter).await?;
        tick.salience_refreshed = true;
    }

    // ── clock 2: formation. One count(*) against the materialize watermark — deliberately cheaper
    // than the load-and-cluster it guards, so below threshold this costs a single query.
    let watermark = shape_watermark(pool, anchor).await?;
    let events = replay::formation_touched_count_since(pool, anchor, watermark).await?;
    let threshold = threshold.unwrap_or(DEFAULT_MATERIALIZE_THRESHOLD);
    if events >= threshold {
        write::incremental_materialize(pool, anchor, lens_name, emitter).await?;
        tick.materialized = true;
    }

    Ok(tick)
}

/// The anchor a resource is homed in — the anchor whose clocks its write ticks.
///
/// `None` when the resource has no home row, which is not an error to escalate: it simply means there
/// is no anchor whose regions this write could affect, so there are no clocks to tick.
pub async fn home_of(
    pool: &sqlx::PgPool,
    resource: uuid::Uuid,
) -> anyhow::Result<Option<HomeAnchor>> {
    let row = sqlx::query!(
        "SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = $1",
        resource,
    )
    .fetch_optional(pool)
    .await?;
    // `from_parts` returns None on an unrecognized discriminant so the call site escalates rather than
    // silently defaulting to the wrong anchor kind — surface that as an error, not a shrug.
    row.map(|r| {
        HomeAnchor::from_parts(&r.anchor_table, r.anchor_id).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown home anchor_table {:?} for {resource}",
                r.anchor_table
            )
        })
    })
    .transpose()
}

/// The anchor's last-materialize watermark — `NULL` before the first materialize, which
/// `formation_touched_count_since` reads as "count everything".
async fn shape_watermark(
    pool: &sqlx::PgPool,
    anchor: HomeAnchor,
) -> anyhow::Result<Option<uuid::Uuid>> {
    // The column is `shape_materialized_event_id` on BOTH anchor tables (T2 gave contexts the same
    // column cogmaps already had), but the table name cannot be bound as a parameter — match to a
    // literal rather than interpolating. The enum is closed, so this is exhaustive by construction.
    let sql = match anchor {
        HomeAnchor::Context(_) => {
            "SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1"
        }
        HomeAnchor::Cogmap(_) => "SELECT shape_materialized_event_id FROM kb_cogmaps WHERE id = $1",
    };
    Ok(sqlx::query_scalar(sql)
        .bind(anchor.uuid())
        .fetch_one(pool)
        .await?)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// A context with an owner and an emitter entity. Empty of resources on purpose: the formation
    /// clock's gate is an event COUNT against the watermark, and an empty context materializes to zero
    /// regions without needing embeddings — so this exercises the gate, not the producer.
    struct Seeded {
        context: Uuid,
        entity: Uuid,
    }

    async fn seed(pool: &PgPool) -> Seeded {
        let owner: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ('owner','owner') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        let context: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
             VALUES ('kb_profiles', $1, 'ctx', 'Ctx') RETURNING id",
        )
        .bind(owner)
        .fetch_one(pool)
        .await
        .unwrap();
        let entity: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name) VALUES ($1, 'e') RETURNING id",
        )
        .bind(owner)
        .fetch_one(pool)
        .await
        .unwrap();
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
}
