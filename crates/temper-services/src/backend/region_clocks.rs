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
