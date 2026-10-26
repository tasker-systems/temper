//! Internal shape-reconcile drain endpoint (task 01a15663). A Vercel cron hits this on a schedule;
//! the handler runs one [`shape_service::dispatch_tick`] pass (reap → claim → verdict each anchor's
//! backlog in bounded batches → complete or hand back) and returns a summary.
//!
//! Same posture as the region drain it mirrors: bearer-secret gated (`require_dispatch_secret`,
//! reusing `EMBED_DISPATCH_SECRET`), fail-closed when unset, no user auth. Reconciliation writes
//! verdicts, a system projection of what the shape in force says about stored content, and takes
//! no request from a caller.
//!
//! **This must be served by the `api/internal` function**, not the public one. The first passes
//! drain whatever backlog accumulated while nothing ran this job, and a pass runs up to its
//! deadline (240 s by default), past the public 60 s `maxDuration`.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use temper_services::error::ApiResult;
use temper_services::services::shape_service::{self, ShapeReconcileSummary};
use temper_services::state::AppState;

use crate::handlers::embed::require_dispatch_secret;

/// Query params for the shape-reconcile drain.
#[derive(Debug, Deserialize)]
pub struct ShapeReconcileQuery {
    /// Max anchors to claim per iteration. Omitted → the service default.
    pub cap: Option<i32>,
}

/// Shape-reconcile drain: `GET`/`POST /api/shape/reconcile`.
///
/// Deliberately out of the OpenAPI contract (plain `.route()`, no `#[utoipa::path]`), the same
/// posture as `region::dispatch` — it is a cron target, not a public API.
pub async fn dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ShapeReconcileQuery>,
) -> ApiResult<Json<ShapeReconcileSummary>> {
    // Fail-closed when unconfigured: no secret ⇒ endpoint disabled.
    require_dispatch_secret(&state, &headers, "shape reconcile")?;

    let summary = shape_service::dispatch_tick(&state.pool, q.cap).await?;
    tracing::info!(
        claimed = summary.claimed,
        completed = summary.completed,
        deferred = summary.deferred,
        partial = summary.partial,
        failed = summary.failed,
        verdicts_written = summary.verdicts_written,
        uncompilable_shapes = summary.uncompilable_shapes,
        "shape reconcile pass complete"
    );
    Ok(Json(summary))
}
