//! Cron-invoked retention sweeps: the Authorization Server tables and the abandoned
//! staged blob uploads.
//!
//! Thin transport over [`as_reap_service::reap_expired_as_rows`] and
//! [`blob_reap_service::reap_abandoned_blob_uploads`]; every retention floor and the
//! reasoning behind it lives in those services, next to the SQL it constrains. The blob
//! sweep is the blob reaper's ONLY driver by ruling (2026-09-05, task 01a0715d): the same
//! cron that sweeps the AS tables carries it, as a sibling pass with the same non-fatal
//! posture — a run that does not happen leaves rows on, which is the fail-safe direction
//! for both domains.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Serialize;

use temper_services::error::ApiResult;
use temper_services::services::as_reap_service::{self, AsReapSummary};
use temper_services::services::blob_reap_service::{self, BlobReapSummary};
use temper_services::state::AppState;

/// What one cron invocation swept, per retention domain.
#[derive(Debug, Serialize)]
pub struct RetentionSweepSummary {
    /// The three Authorization Server tables.
    pub as_tables: AsReapSummary,
    /// Abandoned staged blob-upload sessions.
    pub blob_uploads: BlobReapSummary,
}

/// Cron: sweep expired AS-table rows, then abandoned staged blob uploads.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router, matching the embed
/// crons and the Slack intents reaper. Vercel Cron invokes with GET; POST exists for manual ops —
/// both sweeps are idempotent, so a GET trigger is safe.
///
/// Gated by the shared `EMBED_DISPATCH_SECRET` bearer via `embed::require_dispatch_secret`.
/// **No new secret**, and deliberately: a new
/// fail-closed variable becomes a deploy-time prerequisite, which is the hazard that took the T3
/// deploy dark — see that function's doc comment. Reusing it also means this endpoint inherits the
/// right posture for free, since an unconfigured deploy refuses rather than exposing the sweep.
///
/// The two sweeps are sequential and independent; an error from either fails the run, and
/// the unswept rows simply stay on for the next tick — not reaping is the fail-safe
/// direction for each (see the services). One asymmetry, named for honesty: if the blob
/// sweep errors after the AS sweep ran, the AS sweep's rows are already gone and its
/// summary is lost to the failed response — the harmless direction (those rows wanted
/// sweeping anyway), and the AS sweep's own log line still recorded it.
pub async fn reap_as_tables(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<RetentionSweepSummary>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "AS retention sweep")?;
    let as_tables = as_reap_service::reap_expired_as_rows(&state.pool).await?;
    let blob_uploads = blob_reap_service::reap_abandoned_blob_uploads(&state.pool).await?;
    Ok(Json(RetentionSweepSummary {
        as_tables,
        blob_uploads,
    }))
}
