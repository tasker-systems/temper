//! Cron-invoked check on whether temper-cloud's fail-open internal calls are still reaching us.
//!
//! Thin transport over [`internal_call_health_service::check_internal_call_health`]; the rule that
//! separates a blip from a stopped channel, and the argument for refusing to alarm on silence, live
//! in that service next to the state they decide.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use temper_services::error::ApiResult;
use temper_services::services::internal_call_health_service::{self, InternalCallHealthSummary};
use temper_services::state::AppState;

/// Cron: report each watched internal call channel's health as a span, and as an error event when
/// one has stopped.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router, matching the embed
/// crons, the Slack intents reaper and the AS retention sweep. Vercel Cron invokes with GET; POST
/// exists for manual ops — the check only reads, so a GET trigger is safe.
///
/// Gated by the shared `EMBED_DISPATCH_SECRET` bearer via `embed::require_dispatch_secret`.
/// **No new secret**, for the reason that function's doc comment gives: a new fail-closed variable
/// becomes a deploy-time prerequisite, and the hazard is sharper here than anywhere else in the
/// group. A watcher that is silent because it was never configured reports exactly what a healthy
/// channel reports, and the thing it watches for is silence.
///
/// **Answers 200 whatever it finds**, including a sustained failure. The verdict travels on the
/// span, not the status: a non-2xx would make Vercel record the cron invocation as failed, which
/// says *the check did not run* — the opposite of what a check that ran and found something means.
pub async fn check_internal_calls(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<InternalCallHealthSummary>> {
    crate::handlers::embed::require_dispatch_secret(
        &state,
        &headers,
        "internal call health check",
    )?;
    Ok(Json(
        internal_call_health_service::check_internal_call_health(&state.pool).await?,
    ))
}
