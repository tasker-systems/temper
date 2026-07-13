//! Internal embed-dispatch drain endpoint (issue #299). A Vercel cron POSTs here on a schedule; the
//! handler runs one [`embed_service::dispatch_tick`] pass (reap → claim → embed → complete) and
//! returns a summary. Bearer-secret gated (`EMBED_DISPATCH_SECRET`), fail-closed when unset — a
//! deployment with no drain configured serves 401. No user auth: embedding is a system backfill fed
//! only by the trusted server-side write path.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;

use temper_core::types::admin::{ReembedRequest, ReembedSummary};
use temper_core::types::ids::ProfileId;
use temper_core::types::workflow_job::EmbedDispatchSummary;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::access_service;
use temper_services::services::embed_service::{self, ReembedScope};
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// Query params for the drain: an optional per-tick cap (defaults to the service default) and an
/// optional re-drive flag.
#[derive(Debug, Deserialize)]
pub struct DispatchQuery {
    /// Max resources to embed this pass. Omitted → the service default.
    pub cap: Option<i32>,
    /// When true, re-enqueue `dead` embed jobs before claiming (Phase 4 recovery) so this same pass
    /// drains them. Omitted/false → the normal per-minute cron behavior (no re-drive); recovery is
    /// operator-gated so a persistently-failing resource stays observably `dead`.
    #[serde(default)]
    pub redrive: bool,
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
    params(
        ("cap" = Option<i32>, Query, description = "Max resources to embed this pass"),
        ("redrive" = Option<bool>, Query, description = "Re-enqueue dead embed jobs before claiming (Phase 4 recovery)"),
    ),
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

    let summary = embed_service::dispatch_tick(&state.pool, q.cap, q.redrive).await?;
    tracing::info!(
        redriven = summary.redriven,
        claimed = summary.claimed,
        completed = summary.completed,
        failed = summary.failed,
        chunks = summary.chunks_embedded,
        "embed dispatch pass complete"
    );
    Ok(Json(summary))
}

/// Operator-only re-embed trigger: `POST /api/embed/admin/reembed`.
///
/// Enqueues embed jobs for resources holding **stale** chunks in the requested scope; the per-minute
/// drain then does the work. This endpoint is the *trigger*, never the engine — it returns as soon as
/// the jobs are queued.
///
/// Nothing is marked dirty. Staleness is *derived* (`embedding IS NULL OR embedded_with IS DISTINCT
/// FROM <current model>`), so this is idempotent, safe to re-run, and safe to run while the drain is
/// mid-flight: it simply picks up whatever is still stale. A resource that already has a live job is
/// skipped, so it can never double-queue.
///
/// Admin-gated on the caller's own identity (`is_system_admin`) rather than the drain's shared secret:
/// this is a human operator action, and it should work with the operator's normal login instead of
/// requiring them to hold a deploy secret.
///
/// Deliberately NOT in the OpenAPI contract (plain `.route()`, no `#[utoipa::path]`) — same posture as
/// the rest of the `/api/*/admin/*` surface.
pub async fn reembed(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReembedRequest>,
) -> ApiResult<Json<ReembedSummary>> {
    // Auth before anything else — this can enqueue work across the entire index.
    if !access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await? {
        return Err(ApiError::Forbidden);
    }

    // Exactly one scope. An empty body is a no-op rather than an implicit "everything": the most
    // destructive interpretation must never be the default one.
    let scope = match (body.resource_id, body.context_id, body.all) {
        (Some(id), None, false) => ReembedScope::Resource(id),
        (None, Some(id), false) => ReembedScope::Context(id),
        (None, None, true) => ReembedScope::All,
        _ => {
            return Err(ApiError::BadRequest(
                "specify exactly one of: resource_id, context_id, all".to_string(),
            ))
        }
    };

    let (stale_resources, stale_chunks) = embed_service::stale_summary(&state.pool, scope).await?;

    let enqueued = if body.dry_run {
        Vec::new()
    } else {
        let limit = body.limit.unwrap_or(DEFAULT_REEMBED_LIMIT);
        embed_service::enqueue_stale(&state.pool, scope, limit).await?
    };

    tracing::info!(
        ?scope,
        dry_run = body.dry_run,
        stale_resources,
        stale_chunks,
        enqueued = enqueued.len(),
        "re-embed trigger"
    );

    Ok(Json(ReembedSummary {
        stale_resources,
        stale_chunks,
        enqueued,
    }))
}

/// Default cap on resources enqueued per trigger call. Bounds blast radius: an operator who means to
/// re-embed the whole index walks it in bounded steps rather than queueing thousands of jobs in one go.
const DEFAULT_REEMBED_LIMIT: i32 = 100;
