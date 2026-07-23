//! Evidential-standing service — the one live read over a finding's standing shape.
//!
//! CONFORM to `edge_service` (the sibling `/edges` read, `edge_service.rs`): a thin
//! service-direct wrapper over a `temper_substrate::readback` binding, mapping the
//! substrate-local row to the wire type for the `/api/resources/{id}/evidence` handler.
//!
//! Unlike `edge_service`, this wrapper does NOT pre-gate visibility. The access gate is
//! INSIDE the SQL — `resource_standing_shape`'s `gated` CTE over `resources_readable_by`
//! (`migrations/20260721000010`) — so a principal who cannot read the finding gets zero
//! rows, surfaced by [`readback::resource_standing`] as `None`. That `None` is this read's
//! 404 signal (leak-safe: a denied and an absent finding are indistinguishable). Components
//! are recomputed live by the SQL (never read from the `kb_resource_standing` memo) so
//! `freshness` reflects the current moment.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::error::TemperError;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::standing::StandingShape;
use temper_substrate::readback;

/// Read a finding's evidential-standing shape, scoped to the principal's read access.
///
/// The `id`-bridge: the handler passes bare `Uuid`s (handler-parity with `edge_service`);
/// [`readback::resource_standing`] takes the typed newtypes, so the `Uuid`s convert via their
/// `From<Uuid>` impls (`temper_core::types::ids`). `None` from the readback (unreadable/absent
/// finding — the gate is in the SQL) becomes [`ApiError::NotFound`]; the substrate's
/// `anyhow::Error` maps through `TemperError::Api` like `substrate_read`'s `api_err`.
pub async fn resource_evidence(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<StandingShape> {
    let row = readback::resource_standing(
        pool,
        ProfileId::from(profile_id),
        ResourceId::from(resource_id),
    )
    .await
    .map_err(|e| ApiError::from(TemperError::Api(e.to_string())))?
    .ok_or(ApiError::NotFound)?;

    Ok(StandingShape {
        finding_id: row.finding_id,
        indep_breadth: row.indep_breadth,
        adversarial_survival: row.adversarial_survival,
        challenge_count: row.challenge_count,
        contradiction_balance: row.contradiction_balance,
        freshness: row.freshness,
        r_parent: row.r_parent,
        band: row.band,
    })
}
