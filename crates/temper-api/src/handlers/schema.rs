//! Schema description reads — the vocabularies each kind of work carries.
//!
//! These answer questions about the **embedded schemas**, not about any principal's data:
//! the doc types this binary knows, the fields each requires, the closed vocabularies its
//! fields carry, and the recognized conventions of the open tier. Every answer is
//! identical for every caller.
//!
//! **They are still authenticated, and deliberately so.** They are mounted with the rest
//! of the gated surface — authenticated *and* system-access-gated — rather than beside
//! `/api/health`. Caller-independence is a property of the answer, not a reason to publish
//! it: the MCP door asks `require_profile()` before rendering the same derivation, and a
//! door that dropped the gate because the payload looked harmless would be deciding
//! disclosure by how the bytes read.
//!
//! The derivation itself lives in `temper_workflow::schema`, beside the schemas it reads,
//! and is rendered by the CLI and MCP doors from the same functions. A surface that held
//! its own copy of a vocabulary the system owns is the thing this route exists to make
//! unnecessary.

use axum::extract::Path;
use axum::Json;

use crate::middleware::auth::AuthUser;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_workflow::schema::{DocTypeDescription, DocTypeSummary, OpenMetaConvention};

/// List every document type this instance knows, with a schema summary for each.
#[utoipa::path(
    get,
    operation_id = "list_doc_types",
    path = "/api/schema/doc-types",
    tag = "Schema",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Every known document type", body = Vec<DocTypeSummary>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list_doc_types(_auth: AuthUser) -> ApiResult<Json<Vec<DocTypeSummary>>> {
    Ok(Json(temper_workflow::schema::list_doc_types()))
}

/// Describe one document type: its JSON Schema, required fields, closed vocabularies, and
/// a filled-in example of the managed tier.
///
/// `enum_fields` is the answer to *"which states does this kind of work carry?"* — a task's
/// stages, a goal's statuses — read from the doc-type's own schema rather than from any
/// list a surface keeps.
#[utoipa::path(
    get,
    operation_id = "describe_doc_type",
    path = "/api/schema/doc-types/{name}",
    tag = "Schema",
    params(
        ("name" = String, Path, description = "Document type name (e.g. \"task\", \"goal\")"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The document type's schema, required fields and vocabularies", body = DocTypeDescription),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "No such document type", body = ErrorBody),
    )
)]
pub async fn describe_doc_type(
    _auth: AuthUser,
    Path(name): Path<String>,
) -> ApiResult<Json<DocTypeDescription>> {
    // Not `?` on the workflow error: `DocType::from_str` refuses an unrecognized name with
    // `TemperError::Config`, which maps to a 500. The name came from the caller, so the
    // honest answer is that no such doc type exists — a 404 carrying the enumeration the
    // refusal already names.
    temper_workflow::schema::describe_doc_type(&name)
        .map(Json)
        .map_err(|e| ApiError::NotFound(e.to_string()))
}

/// Describe the recognized conventions of the open (caller-defined) metadata tier.
///
/// The tier stays open — this is guidance, not a closed vocabulary. Each property's
/// `description` states whether the key is FTS-indexed and at what weight; the discouraged
/// keys are surfaced separately because an open tier cannot express their absence.
#[utoipa::path(
    get,
    operation_id = "describe_open_meta",
    path = "/api/schema/open-meta",
    tag = "Schema",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Recognized open_meta conventions and discouraged keys", body = OpenMetaConvention),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn describe_open_meta(_auth: AuthUser) -> ApiResult<Json<OpenMetaConvention>> {
    let convention = temper_workflow::schema::describe_open_meta()
        .map_err(|e| ApiError::Internal(format!("open_meta schema unavailable: {e}")))?;
    Ok(Json(convention))
}
