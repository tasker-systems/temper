use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::authorship::ActContext;
use temper_core::types::citation_audit::{CitationAuditRequest, CitationAuditRow};
use temper_core::types::ids::{BlockId, ProfileId, ResourceId};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::citation_audit_service;
use temper_services::state::AppState;
use temper_workflow::operations::RecordCitationAudit;

// CONFORM to `handlers::edges::assert` (the sibling authored-write handler): thin — build the
// command, dispatch it, map the error. No persistence here.
//
// The real authorization subject is the block's owning finding, resolved server-side from
// `req.block_id` (`temper-services/src/authz/audit_gate.rs:65-77`).
// `citation_audit_service::record_citation_audit` derives that finding and refuses with 404 if
// it disagrees with `id`.
//
// `CitationAuditRequest` carries no act/authorship fields (unlike
// `AssertRelationshipRequest`'s flattened `ActInput`) — that shape was fixed in Task 7/3 and
// is not this task's to change — so the command's `act` is always the empty default here.
/// Record a citation-audit verdict
///
/// `id` is a routing address. The authorization subject is the finding that owns the block named in the request body, resolved server-side.
///
/// If that finding disagrees with `id` the write is refused with 404, so you cannot address one finding in the path while recording an audit against a block of another.
#[utoipa::path(
    post,
    operation_id = "record_citation_audit",
    path = "/api/resources/{id}/citation-audits",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID (the finding being audited)")),
    security(("bearer_auth" = [])),
    request_body = CitationAuditRequest,
    responses(
        (status = 200, description = "Citation audit recorded; returns the new kb_citation_audits.id", body = Uuid),
        (status = 400, description = "Invalid payload, or a verdict value outside [-1.0, 1.0]", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (
            status = 404,
            description = "Not found — the finding is unreadable, the caller authored it (self-audit), or the block belongs to a different finding than {id}",
            body = ErrorBody
        ),
    )
)]
pub async fn record(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(req): Json<CitationAuditRequest>,
) -> ApiResult<Json<Uuid>> {
    let cmd = RecordCitationAudit {
        block: BlockId::from(req.block_id),
        source: req.source,
        value: req.value,
        reason: req.reason,
        act: ActContext::default(),
        origin: surface,
    };
    let audit_id = citation_audit_service::record_citation_audit(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ResourceId::from(resource_id),
        cmd,
    )
    .await?;
    Ok(Json(audit_id))
}

// The `GET` sibling of [`record`] on the same path, and the read that makes an audit
// ATTRIBUTABLE. This read is opt-in rather than more fields on `StandingShape` because the
// shape is fixed-width and recomputed live on every call, while a trail grows with every audit
// ever emitted.
//
// 404-when-unreadable is deliberately not the collection default — the `/provenance` sibling
// answers `200 []` for an unreadable resource. Why this one refuses instead is a leak-safety
// argument about the pair of endpoints, not about this handler: it lives in
// `temper_services::services::citation_audit_service`'s module doc, beside where `/evidence`'s
// equivalent lives in `evidential_standing_service`.
/// List a finding's citation-audit trail
///
/// One row per audit, each naming its auditor.
///
/// `GET /api/resources/{id}/evidence` answers with aggregates only, so a finding disputed by one auditor and a finding disputed by three are indistinguishable there. This read names the voters.
///
/// Answers 404 when the finding is unreadable or absent. An empty array means the finding is readable and genuinely carries no audits.
#[utoipa::path(
    get,
    operation_id = "list_citation_audits",
    path = "/api/resources/{id}/citation-audits",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID (the finding whose audit trail is read)")),
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "The finding's citation audits, most recent first, each attributed to its auditor. Empty when the finding is readable but carries no audits.",
            body = Vec<CitationAuditRow>
        ),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (
            status = 404,
            description = "Not found — the finding is unreadable or does not exist (deliberately indistinguishable, matching GET /evidence)",
            body = ErrorBody
        ),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<Vec<CitationAuditRow>>> {
    citation_audit_service::list_citation_audits(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ResourceId::from(resource_id),
    )
    .await
    .map(Json)
}
