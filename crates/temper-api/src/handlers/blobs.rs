//! Blob surfaces (spec: binary blobs, 2026-09-01 — D6/D7/D9; segment S2 of the surfaces task).
//!
//! `POST /api/blobs` takes one multipart upload at or under the configured single-request
//! threshold (D7 — beyond it, the segmented path of S3); `GET /api/blobs/{id}` streams the
//! bytes read-through with `Cache-Control: immutable` (D6). Both refuse with a disabled
//! refusal when the instance has no blob store configured — absent, not broken, the
//! `NullBroker` posture — and the read renders an invisible blob as 404 so a probe cannot
//! become an existence oracle.

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::header;
use axum::response::Response;
use axum::Json;
use temper_core::types::blob::BlobCommitResponse;
use temper_core::types::ids::{BlobId, ProfileId};
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_substrate::payloads::{AnchorRef, AnchorTable};
use uuid::Uuid;

use crate::middleware::auth::AuthUser;

/// The refusal for an unconfigured instance. Names the vocabulary — the two config postures
/// S1 landed — rather than a bare "unavailable", so an operator knows what enables the door.
fn blob_disabled() -> ApiError {
    ApiError::BadRequest(
        "blob endpoints are disabled — this instance has no blob store configured; set \
         BLOB_STORE_ID (on Vercel, OIDC-first) or BLOB_READ_WRITE_TOKEN (off Vercel) to enable \
         them"
            .to_string(),
    )
}

/// Commit bytes as a blob — one multipart request at or under the D7 threshold
///
/// Form fields: `file` (the bytes; its content type is the media type committed),
/// `home_table` (`kb_contexts` or `kb_cogmaps` — a blob needs a home, D2), `home_id` (the
/// anchor's id). The caller acts as themselves: owner is the authenticated profile.
///
/// The threshold is enforced WHILE the file field streams, so an over-threshold body is
/// refused before it is ever fully buffered, and the refusal names the vocabulary — the
/// threshold in force and the segmented path beyond it. Cap and allowlist stay the SQL
/// wrapper's authority: the substrate write passes this config's numbers through, and a
/// refusal from there surfaces verbatim (`blob_service::map_commit_err`).
#[utoipa::path(
    post,
    operation_id = "commit_blob",
    path = "/api/blobs",
    tag = "Blobs",
    request_body(
        description = "multipart/form-data: `file` (required — the bytes, with a content type), \
                       `home_table` (`kb_contexts` or `kb_cogmaps`), `home_id` (the anchor id)",
        content_type = "multipart/form-data",
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Committed (or dedup-hit) blob", body = BlobCommitResponse),
        (status = 400, description = "Refused — over threshold, unknown home anchor, or the wrapper's cap/allowlist vocabulary", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn commit(
    State(state): State<AppState>,
    auth: AuthUser,
    mut multipart: Multipart,
) -> ApiResult<Json<BlobCommitResponse>> {
    let store = state.blob_store.as_deref().ok_or_else(blob_disabled)?;
    let config = state.config.blob.clone().ok_or_else(blob_disabled)?;
    let caller = ProfileId::from(auth.0.profile().id);

    let mut home_table: Option<String> = None;
    let mut home_id: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("malformed multipart body: {e}")))?
    {
        match field.name() {
            Some("home_table") => {
                home_table = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("bad home_table field: {e}")))?,
                );
            }
            Some("home_id") => {
                home_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("bad home_id field: {e}")))?,
                );
            }
            Some("file") => {
                content_type = field.content_type().map(str::to_string);
                let mut buf: Vec<u8> = Vec::new();
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("bad file field: {e}")))?
                {
                    if buf.len() + chunk.len() > config.single_request_max_bytes {
                        return Err(ApiError::BadRequest(format!(
                            "this request carries {} bytes against a single-request threshold of \
                             {} — beyond it, use the segmented upload path (begin/append/finalize); \
                             the per-blob cap in force is {} bytes",
                            buf.len() + chunk.len(),
                            config.single_request_max_bytes,
                            config.max_bytes
                        )));
                    }
                    buf.extend_from_slice(&chunk);
                }
                bytes = Some(buf);
            }
            _ => {}
        }
    }

    let bytes = bytes
        .ok_or_else(|| ApiError::BadRequest("multipart field `file` is required".to_string()))?;
    let content_bytes = bytes.len() as i64;
    let content_type = content_type.ok_or_else(|| {
        ApiError::BadRequest(
            "multipart field `file` carries no content type — a blob is committed under the \
             media type it is uploaded with"
                .to_string(),
        )
    })?;
    let home = parse_home(home_table, home_id)?;

    let outcome = temper_services::services::blob_service::commit_blob(
        &state.pool,
        store,
        &config,
        caller,
        home,
        content_type.clone(),
        bytes.into(),
    )
    .await?;

    Ok(Json(BlobCommitResponse {
        blob_id: outcome.blob_id,
        content_hash: outcome.content_hash,
        content_type,
        content_bytes,
        deduped: outcome.deduped,
    }))
}

/// Read a blob's bytes back, whole, streamed (D6)
///
/// Visibility gates on the blob's own home via
/// `blob_readable_by_profile` — not visible renders as 404, the same not-found an unknown id
/// gets, so a probe learns nothing either way. The response speaks the STORED media type and
/// carries the byte count plus `Cache-Control: immutable` — content addressing is what earns
/// it (D1), and the provider address never appears anywhere in the response (D6: the API is
/// the only reader of the provider).
#[utoipa::path(
    get,
    operation_id = "get_blob",
    path = "/api/blobs/{id}",
    tag = "Blobs",
    params(
        ("id" = Uuid, Path, description = "Blob ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The blob's bytes, streamed; content type is the stored media type, Cache-Control is immutable", content_type = "application/octet-stream"),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found or not visible — indistinguishable by design", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(blob_id): Path<Uuid>,
) -> ApiResult<Response> {
    let store = state.blob_store.as_deref().ok_or_else(blob_disabled)?;
    let caller = ProfileId::from(auth.0.profile().id);

    let (blob, stream) = temper_services::services::blob_service::read_through(
        &state.pool,
        store,
        caller,
        BlobId::from(blob_id),
    )
    .await?;

    let body = Body::from_stream(stream);
    let mut response = Response::new(body);
    if let Ok(ct) = header::HeaderValue::from_str(&blob.content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, ct);
    }
    if let Ok(cc) = header::HeaderValue::from_str(
        &temper_services::services::blob_service::immutable_cache_control(),
    ) {
        response.headers_mut().insert(header::CACHE_CONTROL, cc);
    }
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        header::HeaderValue::from(blob.content_bytes),
    );
    Ok(response)
}

/// Parse the home form fields into an `AnchorRef`. The wrapper refuses an unknown home table
/// with its own vocabulary; this parse exists because the wire type is an enum — and the
/// refusal mirrors the wrapper's terms (a kb_contexts or kb_cogmaps anchor) so the caller
/// hears one vocabulary regardless of which gate declined.
fn parse_home(home_table: Option<String>, home_id: Option<String>) -> ApiResult<AnchorRef> {
    let table = match home_table.as_deref() {
        Some("kb_contexts") => AnchorTable::Contexts,
        Some("kb_cogmaps") => AnchorTable::Cogmaps,
        other => {
            return Err(ApiError::BadRequest(format!(
                "blob_commit: a blob needs a home (a kb_contexts or kb_cogmaps anchor) — got \
                 home table {}",
                other.unwrap_or("<absent>")
            )))
        }
    };
    let id = home_id
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "blob_commit: home id must be a uuid — got {}",
                home_id.as_deref().unwrap_or("<absent>")
            ))
        })?;
    Ok(AnchorRef { table, id })
}
