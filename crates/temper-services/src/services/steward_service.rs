//! Read path for the team-self-cognition steward's ingest trigger (T4a).
//!
//! Service-direct (the read-path convention): the surface passes a resolved cogmap id + optional
//! threshold; this gates on `anchor_readable_by_profile` and returns the [`IngestDelta`]. The write
//! side (advancing the watermark) routes through `DbBackend`, not here.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::steward::{DriftSweepRow, IngestDelta, DEFAULT_STEWARD_INGEST_THRESHOLD};

/// Compute the ingest delta for a team-self-cognition cogmap: how many new resources + events have
/// landed in the team's contexts since the cogmap's watermark, and whether that clears `threshold`.
///
/// Auth: the caller must be able to READ the cogmap (`anchor_readable_by_profile`). A cogmap the
/// caller cannot see is reported as `NotFound` (show-deny → 404, never leaking existence).
pub async fn ingest_delta(
    pool: &PgPool,
    principal: ProfileId,
    cogmap_id: CogmapId,
    threshold: Option<i64>,
) -> ApiResult<IngestDelta> {
    // One query does the read-gate AND both stored cursors: an absent row means the cogmap does not
    // exist OR the caller cannot read it — both surface as NotFound. Both columns are nullable (NULL
    // watermark = never run; NULL fingerprint = never snapshotted), and they are read together on
    // purpose — the delta compares against BOTH, so a second round trip could only serve a torn pair.
    let cursors = sqlx::query!(
        r#"
        SELECT steward_watermark_event_id  AS "watermark: Uuid",
               steward_boundary_fingerprint AS "stored_fingerprint: String"
          FROM kb_cogmaps
         WHERE id = $1
           AND anchor_readable_by_profile($2, 'kb_cogmaps', $1)
        "#,
        *cogmap_id,
        *principal,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("cognitive map not found or not readable".to_string()))?;
    let watermark = cursors.watermark;

    let row = sqlx::query!(
        r#"
        SELECT new_resources        AS "new_resources!: i64",
               new_events           AS "new_events!: i64",
               max_event_id         AS "max_event_id: Uuid",
               boundary_fingerprint AS "boundary_fingerprint: String",
               boundary_moved       AS "boundary_moved!: bool"
          FROM steward_ingest_delta($1, $2, $3)
        "#,
        *cogmap_id,
        watermark,
        cursors.stored_fingerprint,
    )
    .fetch_one(pool)
    .await?;

    let threshold = threshold.unwrap_or(DEFAULT_STEWARD_INGEST_THRESHOLD);
    Ok(IngestDelta {
        cogmap_id: *cogmap_id,
        watermark,
        new_resources: row.new_resources,
        new_events: row.new_events,
        max_event_id: row.max_event_id,
        threshold,
        // Two arms, OR'd: the counted one, and the boundary one. The boundary arm exists because the
        // counts are taken INSIDE a scope that can itself move silently (a `kb_team_contexts` share
        // emits no event), so a count-only gate under-triggers precisely when the most material
        // changed.
        exceeds_threshold: row.new_resources >= threshold || row.boundary_moved,
        boundary_fingerprint: row.boundary_fingerprint,
        boundary_moved: row.boundary_moved,
    })
}

/// Sweep all team-joined cogmaps the principal can read, returning those whose ingest delta clears
/// `threshold` **or whose change-detection boundary moved**, most-drifted-first. The privileged case
/// (the steward app-principal) simply has broad read; the gate is the same
/// `anchor_readable_by_profile` every read uses — not a bypass.
///
/// The boundary arm is the same OR that `ingest_delta`'s `exceeds_threshold` carries, and it lives in
/// `steward_drift_sweep`'s own WHERE rather than being re-derived here: the cron path and the
/// per-cogmap path must not be able to disagree about what "drifted" means.
pub async fn drift_sweep(
    pool: &PgPool,
    principal: ProfileId,
    threshold: Option<i64>,
) -> ApiResult<Vec<DriftSweepRow>> {
    let threshold = threshold.unwrap_or(DEFAULT_STEWARD_INGEST_THRESHOLD);
    let rows = sqlx::query!(
        r#"
        SELECT cogmap_id      AS "cogmap_id!: Uuid",
               watermark      AS "watermark: Uuid",
               new_resources  AS "new_resources!: i64",
               new_events     AS "new_events!: i64",
               boundary_moved AS "boundary_moved!: bool"
          FROM steward_drift_sweep($1, $2)
        "#,
        *principal,
        threshold,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DriftSweepRow {
            cogmap_id: r.cogmap_id,
            watermark: r.watermark,
            new_resources: r.new_resources,
            new_events: r.new_events,
            boundary_moved: r.boundary_moved,
        })
        .collect())
}

/// All team-joined cogmaps the principal can AUTHOR (the materialize fan-out candidate set).
///
/// Authorable, not merely readable: this feeds the hourly materialize cron, whose
/// `materialize_on_threshold` gate is `cogmap_authorable_by_profile` — an explicit write grant. A
/// readable-but-unauthorable map handed out here is a guaranteed 403, and the cron fans out with
/// `Promise.all` and throws on any non-ok, so ONE such map fails the entire tick. The L0 kernel is
/// exactly that map for every principal (root-team readable, authorable by nobody by design), which
/// is why it has never once materialized.
///
/// The auditor's candidate set is a different question with a different answer and deliberately still
/// calls `steward_candidate_cogmaps` — see `20260805000010`'s header.
pub async fn candidate_cogmaps(pool: &PgPool, principal: ProfileId) -> ApiResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar!(
        r#"SELECT cogmap_id AS "id!: Uuid" FROM steward_authorable_cogmaps($1)"#,
        *principal,
    )
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
