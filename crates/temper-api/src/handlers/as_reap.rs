//! Cron-invoked retention sweep for the Authorization Server tables.
//!
//! Thin transport over [`as_reap_service::reap_expired_as_rows`]; every retention floor and the
//! reasoning behind it lives in that service, next to the SQL it constrains.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use temper_services::error::ApiResult;
use temper_services::services::as_reap_service::{self, AsReapSummary};
use temper_services::state::AppState;

/// Cron: sweep expired rows from `kb_saml_replay`, `kb_oauth_flow` and `kb_oauth_refresh_tokens`.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router, matching the embed
/// crons and the Slack intents reaper. Vercel Cron invokes with GET; POST exists for manual ops —
/// the sweep is idempotent, so a GET trigger is safe.
///
/// Gated by the shared `EMBED_DISPATCH_SECRET` bearer via `embed::require_dispatch_secret`.
/// **No new secret**, and deliberately: a new
/// fail-closed variable becomes a deploy-time prerequisite, which is the hazard that took the T3
/// deploy dark — see that function's doc comment. Reusing it also means this endpoint inherits the
/// right posture for free, since an unconfigured deploy refuses rather than exposing the sweep.
pub async fn reap_as_tables(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<AsReapSummary>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "AS retention sweep")?;
    Ok(Json(
        as_reap_service::reap_expired_as_rows(&state.pool).await?,
    ))
}
