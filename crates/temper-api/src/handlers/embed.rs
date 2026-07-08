//! Internal embed-dispatch drain endpoint (issue #299). A Vercel cron POSTs here on a schedule; the
//! handler runs one [`embed_service::dispatch_tick`] pass (reap → claim → embed → complete) and
//! returns a summary. Bearer-secret gated (`EMBED_DISPATCH_SECRET`), fail-closed when unset — a
//! deployment with no drain configured serves 401. No user auth: embedding is a system backfill fed
//! only by the trusted server-side write path.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use temper_core::types::workflow_job::EmbedDispatchSummary;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::embed_service;
use temper_services::state::AppState;

/// Query params for the drain: an optional per-tick cap (defaults to the service default).
#[derive(Debug, Deserialize)]
pub struct DispatchQuery {
    /// Max resources to embed this pass. Omitted → the service default.
    pub cap: Option<i32>,
}

/// Constant-time equality over the presented bearer token and the configured secret — avoids a
/// byte-by-byte early-exit timing oracle on the shared secret.
fn secret_matches(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// GET (not POST): Vercel Cron invokes its target with a GET carrying the `CRON_SECRET` as a bearer.
// The route also accepts POST for manual ops. The pass is effectively idempotent — a re-run just
// claims whatever is still pending — so a GET trigger is safe here.
#[utoipa::path(
    get,
    path = "/api/embed/dispatch",
    tag = "Embed",
    params(("cap" = Option<i32>, Query, description = "Max resources to embed this pass")),
    responses((status = 200, description = "One embed-dispatch pass summary", body = EmbedDispatchSummary)),
)]
pub async fn dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DispatchQuery>,
) -> ApiResult<Json<EmbedDispatchSummary>> {
    // Fail-closed when unconfigured: no secret ⇒ endpoint disabled.
    let expected = match state.config.embed_dispatch_secret.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!("embed dispatch: rejected (endpoint disabled — secret unset)");
            return Err(ApiError::Unauthorized(
                "embed dispatch disabled".to_string(),
            ));
        }
    };
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !secret_matches(presented, expected) {
        tracing::warn!("embed dispatch: rejected (bad or missing bearer secret)");
        return Err(ApiError::Unauthorized(
            "invalid embed dispatch secret".to_string(),
        ));
    }

    let summary = embed_service::dispatch_tick(&state.pool, q.cap).await?;
    tracing::info!(
        claimed = summary.claimed,
        completed = summary.completed,
        failed = summary.failed,
        chunks = summary.chunks_embedded,
        "embed dispatch pass complete"
    );
    Ok(Json(summary))
}
