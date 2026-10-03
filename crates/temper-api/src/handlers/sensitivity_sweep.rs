//! Cron-invoked sensitivity sweep tick (sensitivity-sweep spec D8; build order 3a PR D).
//!
//! Thin transport over [`sensitivity_sweep_service::sweep`]. Why the claim and the tick are two
//! statements, why nothing in the response is text, and why a missing salt is loud are all argued
//! in that service, next to the code they decide.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use temper_services::error::ApiResult;
use temper_services::services::sensitivity_sweep_service::{self, SweepAnswer};
use temper_services::state::AppState;

/// Cron: claim and scan surfaces' work orders until the call's budget is spent or a rotation is idle.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router, like the other
/// crons in `embed_internal_routes`. Vercel Cron invokes with GET; POST exists for manual ops. A
/// re-run only claims whatever is next, so a GET trigger is safe.
///
/// Gated by the shared `EMBED_DISPATCH_SECRET` bearer via `embed::require_dispatch_secret`: no new
/// gate secret (D8). The salt is a key, not a gate, and has its own variable (Q44).
///
/// **Answers 200 whatever the ticks found**, including a failed tick: the verdict travels on the
/// spans and the run rows. A tick whose statement raises is the exception: the call stops there
/// and answers 500, after its span and error event, because the cron did not finish its work.
///
/// The answer is counts of ticks and booleans. The per-tick counts stay on the span: a caller who
/// can plant a unit and read `cache_hits` back would learn whether that unit exists anywhere.
pub async fn sweep(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SweepAnswer>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "sensitivity sweep")?;
    let salt = state
        .config
        .sensitivity_sweep_salt
        .as_deref()
        .map(str::as_bytes);
    Ok(Json(
        sensitivity_sweep_service::sweep(&state.pool, salt)
            .await?
            .answer(),
    ))
}
