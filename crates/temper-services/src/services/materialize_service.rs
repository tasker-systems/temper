//! Read path for cron-driven region materialization on a drift threshold (T4b).
//!
//! Service-direct (the read-path convention): the surface passes a resolved anchor + optional
//! threshold; this gates on `anchor_readable_by_profile` and returns the [`MaterializeDelta`]. The
//! write side (running the materialize when over threshold) routes through `DbBackend` /
//! `DbBackend`, not here.
//!
//! The delta reuses the SAME drift signal as the "is a recorded fingerprint stale?" gate — a cheap
//! `count(*)` of formation events since `shape_materialized_event_id` — deliberately cheaper than the
//! materialize it guards.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ProfileId;
use temper_core::types::materialize::{MaterializeDelta, DEFAULT_MATERIALIZE_THRESHOLD};

/// Compute the materialize delta for an anchor: how many formation events have landed on it since it
/// was last materialized, and whether that clears `threshold`.
///
/// Anchor-generic because everything beneath it already was — `formation_touched_count_since` takes a
/// [`HomeAnchor`], and `shape_materialized_event_id` is a column on BOTH anchor tables. Only this
/// function's signature and its inline gate query were cogmap-only.
///
/// Auth: the caller must be able to READ the anchor (`anchor_readable_by_profile`). An anchor the
/// caller cannot see is reported as `NotFound` (show-deny → 404, never leaking existence).
///
/// **This surface's deny posture is 404 and stays 404.** The shape read next door denies with an
/// empty envelope and never an error (`crate::backend::substrate_read::anchor_shape_select`). Both
/// are non-oracular; they are not the same rule, and neither posture travels to the other.
pub async fn materialize_delta(
    pool: &PgPool,
    principal: ProfileId,
    anchor: HomeAnchor,
    threshold: Option<i64>,
) -> ApiResult<MaterializeDelta> {
    // One query does the read-gate AND the materialize-watermark lookup: an absent row means the
    // anchor does not exist OR the caller cannot read it — both surface as NotFound. The column is
    // nullable (NULL = never materialized), so the scalar is `Option<Uuid>` inside the fetch_optional
    // `Option`.
    //
    // The table name cannot be bound as a parameter, so the statement goes inside each match arm
    // rather than being selected into one runtime call — the same shape `region_clocks::shape_watermark`
    // uses for the identical column. Both literals land in the `.sqlx` cache; a query chosen at
    // runtime from a closed set is still two static queries, and the enum keeps it exhaustive.
    let id = anchor.uuid();
    let watermark: Option<Uuid> = match anchor {
        HomeAnchor::Context(_) => {
            sqlx::query_scalar!(
                r#"
                SELECT shape_materialized_event_id AS "watermark: Uuid"
                  FROM kb_contexts
                 WHERE id = $1
                   AND anchor_readable_by_profile($2, 'kb_contexts', $1)
                "#,
                id,
                *principal,
            )
            .fetch_optional(pool)
            .await?
        }
        HomeAnchor::Cogmap(_) => {
            sqlx::query_scalar!(
                r#"
                SELECT shape_materialized_event_id AS "watermark: Uuid"
                  FROM kb_cogmaps
                 WHERE id = $1
                   AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
                "#,
                id,
                *principal,
            )
            .fetch_optional(pool)
            .await?
        }
    }
    .ok_or_else(|| {
        // The noun follows the anchor: a context denied by this path must not be called a cognitive
        // map. The two arms say the same amount — existence and readability stay collapsed.
        let noun = match anchor {
            HomeAnchor::Context(_) => "context",
            HomeAnchor::Cogmap(_) => "cognitive map",
        };
        ApiError::NotFound(format!("{noun} not found or not readable"))
    })?;

    let formation_events =
        temper_substrate::replay::formation_touched_count_since(pool, anchor, watermark)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    let threshold = threshold.unwrap_or(DEFAULT_MATERIALIZE_THRESHOLD);
    Ok(MaterializeDelta::new(
        anchor,
        watermark,
        formation_events,
        threshold,
    ))
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
