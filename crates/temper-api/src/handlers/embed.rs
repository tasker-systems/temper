//! Internal embed-dispatch drain endpoint (issue #299). A Vercel cron POSTs here on a schedule; the
//! handler runs one [`embed_service::dispatch_tick`] pass (reap → claim → embed → complete) and
//! returns a summary. Gated by the `CronSecret` tier (`middleware::cron_secret`): pinned to the
//! deployment's own host on Vercel, then bearer-secret gated (`EMBED_DISPATCH_SECRET`), fail-closed
//! when unset. No user auth: embedding is a system backfill fed
//! only by the trusted server-side write path.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use temper_core::types::admin::{ReembedRequest, ReembedSummary};
use temper_core::types::workflow_job::EmbedDispatchSummary;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
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
    /// Cosmetic shard id — distinguishes the N cron lines that fan the drain out. Carries NO logic:
    /// concurrent drainers partition the queue via the claim's `FOR UPDATE SKIP LOCKED`, so this is
    /// only echoed into the trace for per-shard observability. See the throughput-scaling spec.
    pub shard: Option<i32>,
}

// GET (not POST): Vercel Cron invokes its target with a GET carrying the `CRON_SECRET` as a bearer.
// The route also accepts POST for manual ops. The pass is effectively idempotent — a re-run just
// claims whatever is still pending — so a GET trigger is safe here.
/// Claim embed jobs for a dispatch pass
#[utoipa::path(
    get,
    path = "/api/embed/dispatch",
    tag = "Embed",
    params(
        ("cap" = Option<i32>, Query, description = "Max resources to embed this pass"),
        ("redrive" = Option<bool>, Query, description = "Re-enqueue dead embed jobs before claiming (Phase 4 recovery)"),
        // `shard` is intentionally NOT documented here: it is a cosmetic, cron-internal fan-out knob
        // (SKIP LOCKED does the partitioning, not the value), not part of the public contract.
    ),
    responses((status = 200, description = "One embed-dispatch pass summary", body = EmbedDispatchSummary)),
)]
pub async fn dispatch(
    State(state): State<AppState>,
    Query(q): Query<DispatchQuery>,
) -> ApiResult<Json<EmbedDispatchSummary>> {
    let summary = embed_service::dispatch_tick(&state.pool, q.cap, q.redrive).await?;
    tracing::info!(
        shard = q.shard,
        redriven = summary.redriven,
        claimed = summary.claimed,
        completed = summary.completed,
        failed = summary.failed,
        chunks = summary.chunks_embedded,
        "embed dispatch pass complete"
    );
    Ok(Json(summary))
}

/// One-line summary of an embedder-warm pass, returned by [`warm`].
#[derive(Debug, Serialize)]
pub struct WarmSummary {
    /// Always `true` on success — the embedder loaded and produced a vector.
    warmed: bool,
    /// Embedding dimensionality (768 for bge-base) — a liveness signal that inference really ran.
    dims: usize,
    /// Wall-clock the warm took. The FIRST call on a cold instance pays the full model load — the
    /// cost this endpoint exists to move OFF the search request path; later calls on the same warm
    /// instance are a cheap cached inference.
    ms: u64,
}

// GET (not POST): Vercel Cron invokes its target with a GET carrying the secret as a bearer, same as
// `dispatch`.
/// Cold-start warmup for server-side query embedding: `GET /api/embed/warm`.
///
/// A search whose caller can't precompute an embedding (MCP, web UI, `--text-only`) is embedded
/// server-side inside the request. On a cold instance that pays a one-time ONNX model load which, run
/// inline, can exceed the query-embed budget (`TEMPER_QUERY_EMBED_BUDGET_MS`, default 8s) and silently
/// drop the vector arm (issue #427). This endpoint loads and exercises the embedder so the process's
/// model cache is hot; a periodic cron (`vercel.json`) keeps a warm instance on a low-traffic deploy.
/// In the Vercel deploy it is routed to the dedicated `api/internal` function (alongside `dispatch`),
/// so it keeps the **embed-drain worker** hot — where the repeated ONNX cost lives post-#299; the
/// public search paths rely on the memory→CPU lever plus the graceful FTS degrade above. In a
/// single-process deploy (local, self-hosted) it warms the one process that serves everything.
///
/// Same gate as [`dispatch`], the `CronSecret` tier (`EMBED_DISPATCH_SECRET`), fail-closed when
/// unset — the warm path runs ONNX and must never be an open trigger. Idempotent and cheap after the first load.
/// Deliberately OUT of the OpenAPI contract (plain `.route()`, no `#[utoipa::path]`), same posture as
/// `dispatch`.
pub async fn warm() -> ApiResult<Json<WarmSummary>> {
    let started = std::time::Instant::now();
    let dims = embed_service::warm_embedder().await?;
    let ms = started.elapsed().as_millis() as u64;

    tracing::info!(dims, ms, "embed warm pass complete");
    Ok(Json(WarmSummary {
        warmed: true,
        dims,
        ms,
    }))
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
/// Documented under the `Admin` tag with the rest of the operator surface (`routes/admin.rs`). The
/// drain and warm crons beside it stay out of the contract: their only caller holds a deploy secret.
#[utoipa::path(
    post,
    operation_id = "admin_reembed",
    summary = "Re-embed stale chunks",
    description = "Enqueues embedding jobs for chunks whose vector is missing or came from a model the server no longer embeds with, scoped to one resource, one context, or everything. `dry_run` reports what is stale without enqueuing. Bounded by `limit` per call and safe to repeat. Requires a system admin.",
    path = "/api/embed/admin/reembed",
    tag = "Admin",
    request_body = ReembedRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "What is stale in scope, and the resources enqueued (none on `dry_run`). Resources homed in personal space are enqueued by the `all` sweep but not named", body = ReembedSummary),
        (status = 400, description = "Not exactly one of `resource_id`, `context_id`, `all`", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is not a system admin, the scope is someone else's personal context or a resource homed in one (`FORBIDDEN_DETAIL`: this context is, or this resource is homed in, @handle's personal space; the admin arm does not reach it), or lacks system access (`SYSTEM_ACCESS_REQUIRED`); or the caller is a machine principal (`FORBIDDEN_DETAIL`: this action is not available to a machine principal)", body = ErrorBody),
    )
)]
pub async fn reembed(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ReembedRequest>,
) -> ApiResult<Json<ReembedSummary>> {
    // Auth before anything else — this can enqueue work across the entire index. The `&SystemAdmin`
    // proof required by `enqueue_stale` is the gate (admin-authz enclosure, spec §3); minted here.
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;

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

    // A system admin does not aim at personal space (task `01a191b4`). Refused before anything is
    // read, so the refusal says nothing about what is stale there.
    // The `all` sweep still re-embeds personal space, without naming it.
    if let Some(refusal) = embed_service::personal_scope_refusal(&state.pool, &admin, scope).await?
    {
        return Err(refusal);
    }

    let (stale_resources, stale_chunks) = embed_service::stale_summary(&state.pool, scope).await?;

    let enqueued = if body.dry_run {
        Vec::new()
    } else {
        let limit = body.limit.unwrap_or(DEFAULT_REEMBED_LIMIT);
        let enqueued = embed_service::enqueue_stale(&state.pool, &admin, scope, limit).await?;
        embed_service::unnamed_personal(&state.pool, &admin, enqueued).await?
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shard param is cosmetic — it distinguishes the N fan-out cron lines and is echoed to the
    /// trace, never used for logic (drainers partition via SKIP LOCKED). It must deserialize from the
    /// query string, and its absence must remain valid (the single-cron and manual-trigger cases).
    #[test]
    fn dispatch_query_accepts_optional_shard() {
        let with: DispatchQuery = serde_urlencoded::from_str("shard=3&cap=5").unwrap();
        assert_eq!(with.shard, Some(3));
        assert_eq!(with.cap, Some(5));

        let without: DispatchQuery = serde_urlencoded::from_str("").unwrap();
        assert_eq!(without.shard, None);
        assert!(!without.redrive);
    }

    /// The warm summary is the cron/operator-facing JSON; pin its field names so a rename that would
    /// break a monitoring probe is caught here rather than in production.
    #[test]
    fn warm_summary_serializes_expected_fields() {
        let json = serde_json::to_value(WarmSummary {
            warmed: true,
            dims: 768,
            ms: 1234,
        })
        .expect("serialize WarmSummary");
        assert_eq!(json["warmed"], serde_json::json!(true));
        assert_eq!(json["dims"], serde_json::json!(768));
        assert_eq!(json["ms"], serde_json::json!(1234));
    }
}
