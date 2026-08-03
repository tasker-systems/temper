//! Internal region-clock drain endpoint (goal 019fc46c). A Vercel cron hits this on a schedule; the
//! handler runs one [`region_service::dispatch_tick`] pass (reap → claim → tick → complete) and
//! returns a summary.
//!
//! Same posture as the embed drain it mirrors: bearer-secret gated
//! (`require_dispatch_secret`, reusing `EMBED_DISPATCH_SECRET`), fail-closed when unset, no user
//! auth — region materialization is a system projection fed only by the trusted server-side write
//! path. Reusing the existing secret rather than introducing a second one is deliberate: a new
//! fail-closed env var becomes a deploy-time prerequisite, which is the hazard that took the T3
//! deploy dark.
//!
//! **This must be served by the `api/internal` function**, not the public one. A single settling has
//! been observed at 55–94s in production, which exceeds the public 60s `maxDuration` outright.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use temper_services::error::ApiResult;
use temper_services::services::region_service::{self, RegionDispatchSummary};
use temper_services::state::AppState;

use crate::handlers::embed::require_dispatch_secret;

/// Query params for the region drain.
#[derive(Debug, Deserialize)]
pub struct RegionDispatchQuery {
    /// Max anchors to claim per iteration. Omitted → the service default.
    pub cap: Option<i32>,
}

/// Region-clock drain: `GET`/`POST /api/region/dispatch`.
///
/// Deliberately out of the OpenAPI contract (plain `.route()`, no `#[utoipa::path]`), same posture as
/// `embed::dispatch` — it is a cron target, not a public API.
pub async fn dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RegionDispatchQuery>,
) -> ApiResult<Json<RegionDispatchSummary>> {
    // Fail-closed when unconfigured: no secret ⇒ endpoint disabled.
    require_dispatch_secret(&state, &headers, "region dispatch")?;

    let summary = region_service::dispatch_tick(&state.pool, q.cap).await?;
    tracing::info!(
        claimed = summary.claimed,
        completed = summary.completed,
        deferred = summary.deferred,
        failed = summary.failed,
        materialized = summary.materialized,
        salience_refreshed = summary.salience_refreshed,
        "region dispatch pass complete"
    );
    Ok(Json(summary))
}
