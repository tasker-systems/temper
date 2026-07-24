use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::authorship::ActContext;
use temper_core::types::citation_audit::CitationAuditRequest;
use temper_core::types::ids::{BlockId, ProfileId, ResourceId};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::citation_audit_service;
use temper_services::state::AppState;
use temper_workflow::operations::RecordCitationAudit;

/// Record an auditor's signed defensibility verdict on one `(block, source)` citation of the
/// finding at `{id}`. CONFORM to `handlers::edges::assert` (the sibling authored-write handler):
/// thin — build the command, dispatch it, map the error. No persistence here.
///
/// `id` is a routing address only. The real authorization subject is the block's owning finding,
/// resolved server-side from `req.block_id`
/// (`temper-services/src/authz/audit_gate.rs:65-77`). `citation_audit_service::record_citation_audit`
/// derives that finding and refuses with 404 if it disagrees with `id`, so a caller cannot address
/// one finding in the path while writing an audit onto a block of another.
///
/// `CitationAuditRequest` carries no act/authorship fields (unlike `AssertRelationshipRequest`'s
/// flattened `ActInput`) — that shape was fixed in Task 7/3 and is not this task's to change — so
/// the command's `act` is always the empty default here.
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
