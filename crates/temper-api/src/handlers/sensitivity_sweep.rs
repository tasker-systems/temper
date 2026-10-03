//! Cron-invoked sensitivity sweep tick (sensitivity-sweep spec D8; build order 3a PR D).
//!
//! Thin transport over [`sensitivity_sweep_service::sweep`]. Why the claim and the tick are two
//! statements, why nothing in the response is text, and why a missing salt is loud are all argued
//! in that service, next to the code they decide.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use temper_services::error::ApiResult;
use temper_services::services::sensitivity_sweep_service::{self, SweepSummary};
use temper_services::state::AppState;

/// Cron: claim one surface's work order and scan it.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router, like the other
/// crons in `embed_internal_routes`. Vercel Cron invokes with GET; POST exists for manual ops. A
/// re-run only claims whatever is next, so a GET trigger is safe.
///
/// Gated by the shared `EMBED_DISPATCH_SECRET` bearer via `embed::require_dispatch_secret`: no new
/// gate secret (D8). The salt is a key, not a gate, and has its own variable (Q44).
///
/// **Answers 200 whatever the tick found**, including a failed tick. The verdict travels on the
/// span and the run row: a non-2xx would make Vercel record that the cron did not run, which is the
/// opposite of a tick that ran and failed.
pub async fn sweep(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SweepSummary>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "sensitivity sweep")?;
    let salt = state
        .config
        .sensitivity_sweep_salt
        .as_deref()
        .map(str::as_bytes);
    Ok(Json(
        sensitivity_sweep_service::sweep(&state.pool, salt).await?,
    ))
}
