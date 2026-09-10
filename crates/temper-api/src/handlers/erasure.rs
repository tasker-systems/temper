//! The erasure act's two HTTP surfaces (spec 2026-08-31, task 01a0577c Beat 4): the operator's
//! execute door and the byte-delete fence's cron tick.
//!
//! **The door is gate-free by ruling.** Authorization lives in the SERVICE
//! (`erasure_service::execute_erasure` resolves `is_system_admin` before any mutation), and the
//! door must not pre-empt or duplicate that gate — a prelude here would decide legality twice
//! and could drift from the refusal the service records. The door's one job is the POSTURE:
//! a non-operator's attempt is answered **404, never 403** (the admin-ledger pattern,
//! `handlers/admin_ledger.rs` — a 403 would confirm an erasure door exists and who it refuses),
//! while the service has already recorded the `unauthorized` refusal. Mounted plain
//! (`.route()`), out of the OpenAPI contract like `/api/admin/ledger`; allowlisted in
//! `.github/scripts/check-openapi-routes.sh`.
//!
//! **The drain is the fence's only driver.** Same internal-cron posture as
//! `/api/embed/dispatch`: bearer-gated by the shared `EMBED_DISPATCH_SECRET` (no new secret —
//! the fail-closed-variable hazard `require_dispatch_secret` exists to avoid), GET+POST on one
//! handler, excluded from the contract entirely.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::erasure_fence_service::{self, DrainSummary};
use temper_services::services::erasure_service::{self, ErasureOutcome};
use temper_services::state::AppState;
use temper_substrate::payloads::{ErasureRefusalReason, ErasureTargetOutcome};

use crate::middleware::auth::AuthUser;

/// The execute door's request: the subject as the pseudonym UUID, plus the opaque request
/// reference (UUID — the `RefRel::Request` apparatus Beat 2 pinned). No name, no email, no case
/// description: the request-to-person mapping lives in the operator's DSAR records, outside the
/// ledger.
#[derive(Debug, Deserialize)]
pub struct ErasureExecuteRequest {
    pub subject: Uuid,
    pub request_reference: Uuid,
}

/// One blob strike of a completed erasure, as the door reports it.
#[derive(Debug, Serialize)]
pub struct BlobStrikeView {
    pub blob_id: Uuid,
    pub released: bool,
}

/// What the door's act did. A tagged enum, not optional fields: a completion and a refusal are
/// different answers to different questions, and collapsing them into one shape makes "which
/// happened?" a matter of which fields are null.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ErasureExecuteResponse {
    /// The act completed — in full, or as the no-op completion on an already-erased subject.
    Completed {
        event_id: Uuid,
        already_erased: bool,
        /// The redacted set (D2): content hashes only.
        redacted_hashes: Vec<String>,
        /// Per-target outcomes and the named remainder (D6's accepted-in-part arm): the
        /// operator sees the `independent_obligation` remainder AT THE DOOR, not only in the
        /// ledger — the completion's own payload is the audit, but the door's caller is the
        /// actor and deserves the same facts.
        targets: Vec<ErasureTargetOutcome>,
        blob_strikes: Vec<BlobStrikeView>,
    },
    /// The act was refused and the refusal RECORDED (D6). `unauthorized` never reaches the
    /// wire — the door answers it with 404 before serializing anything.
    Refused {
        event_id: Uuid,
        reason: ErasureRefusalReason,
        detail: Option<String>,
    },
}

/// `POST /api/admin/erasure` — the operator's execute door.
///
/// Gate-free HERE on purpose: the service's `is_system_admin` gate is the authority (it runs
/// first, before any mutation, and records the `unauthorized` refusal for a non-operator). This
/// handler maps that refusal to the 404 posture and never re-asks the question.
///
/// The request reference tolerates retries: a retried POST with the SAME reference re-executes
/// as a no-op completion (the subject is already tombstoned, so the act completes with
/// `already_erased: true`). Correlation is INDEXED, never unique
/// (20260624000001_canonical_schema.sql:491) — the reference pairs the act's events, it does
/// not deduplicate the door.
pub async fn execute(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ErasureExecuteRequest>,
) -> ApiResult<Json<ErasureExecuteResponse>> {
    let caller = ProfileId::from(auth.0.profile().id);
    let outcome = erasure_service::execute_erasure(
        &state.pool,
        caller,
        ProfileId::from(body.subject),
        body.request_reference,
    )
    .await?;

    match outcome {
        // The gate's refusal face: the operator-only door renders ABSENT (404) — a 403 would
        // disclose that an erasure door exists and that this caller was refused by it. The
        // refusal event is already on the ledger; nothing is said to the caller.
        ErasureOutcome::Refused(r) if r.reason == ErasureRefusalReason::Unauthorized => {
            Err(ApiError::NotFound("not found".to_string()))
        }
        // An operator-facing refusal (unhonourable scope, independent obligation) is the
        // operator's own information: the door answers it plainly. Unreachable through this
        // door today — the service's only pre-gate refusal is `unauthorized` — but the arm is
        // the D6 face rendered honestly rather than an internal error.
        ErasureOutcome::Refused(r) => Ok(Json(ErasureExecuteResponse::Refused {
            event_id: r.event_id,
            reason: r.reason,
            detail: r.detail,
        })),
        ErasureOutcome::Completed(c) => Ok(Json(ErasureExecuteResponse::Completed {
            event_id: c.event_id,
            already_erased: c.already_erased,
            redacted_hashes: c.redacted_hashes,
            targets: c.targets,
            blob_strikes: c
                .blob_strikes
                .into_iter()
                .map(|s| BlobStrikeView {
                    blob_id: s.blob_id,
                    released: s.released,
                })
                .collect(),
        })),
    }
}

/// The fence drain's response: the tick's tallies plus whether a provider is configured.
#[derive(Debug, Serialize)]
pub struct FenceDrainResponse {
    #[serde(flatten)]
    pub drain: DrainSummary,
    pub store_configured: bool,
}

/// `GET|POST /api/erasure/drain` — one byte-delete fence tick.
///
/// Seeding is STORE-INDEPENDENT: the pending deletes are derived from the ledger, never from a
/// provider, so a tick with no provider config still seeds every erasure's released strikes and
/// still ages them into the alertable state ([`erasure_fence_service::drain_without_store`]) —
/// the substrate contract's stranding posture ("retry plus age alerting") has no
/// store-configured precondition, and a deployment that HAD a provider and lost its
/// configuration must go LOUD about its stranded deletes, never quiet. Only the provider CALLS
/// (claim → delete → resolve) require the store.
pub async fn drain(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<FenceDrainResponse>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "erasure delete drain")?;

    match state.blob_store.as_deref() {
        Some(store) => {
            let summary = erasure_fence_service::drain(&state.pool, store).await?;
            tracing::info!(
                seeded = summary.seeded,
                unparseable_verdicts = summary.unparseable_verdicts,
                claimed = summary.claimed,
                deleted = summary.deleted,
                skipped = summary.skipped_reoccupied,
                failed = summary.failed,
                "erasure fence drain pass complete"
            );
            Ok(Json(FenceDrainResponse {
                drain: summary,
                store_configured: true,
            }))
        }
        None => {
            let summary = erasure_fence_service::drain_without_store(&state.pool).await?;
            tracing::info!(
                seeded = summary.seeded,
                unparseable_verdicts = summary.unparseable_verdicts,
                store_configured = false,
                "erasure fence seed-only pass complete (no provider configured)"
            );
            Ok(Json(FenceDrainResponse {
                drain: summary,
                store_configured: false,
            }))
        }
    }
}
