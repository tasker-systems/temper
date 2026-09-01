//! Blob surfaces (spec: binary blobs, 2026-09-01 — D6/D7/D9; segments S2/S3 of the
//! surfaces task).
//!
//! `POST /api/blobs` takes one multipart upload at or under the configured single-request
//! threshold (D7 — beyond it, the segmented path below); `GET /api/blobs/{id}` streams the
//! bytes read-through with `Cache-Control: immutable` (D6). The segmented path
//! (`/api/blobs/uploads/*`) stages bytes in Postgres between begin and finalize — pre-ledger
//! transport state, caller-private until finalized, never a blob until the hash exists. Both
//! paths refuse with a disabled refusal when the instance has no blob store configured —
//! absent, not broken, the `NullBroker` posture — and the read renders an invisible blob as
//! 404 so a probe cannot become an existence oracle.

use axum::body::{Body, Bytes};
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use temper_core::types::blob::{
    BlobCommitResponse, BlobUploadBeginRequest, BlobUploadBeginResponse, BlobUploadFinalizeRequest,
    BlobUploadProgress,
};
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

// ── Segmented upload (S3, D7) ─────────────────────────────────────────────────────
// Thin handlers, the `segments.rs` shape: AuthUser extractor → service → ApiError. The
// gates live in the service (owner-equality on the session row; the F-2 standing two-step
// at begin and again at finalize); the wrapper stays the sole cap/allowlist authority.

/// The home-table parse for the segmented begin, in the S2 `parse_home` terms: the wire
/// type is an enum, and the refusal mirrors the wrapper's vocabulary so the caller hears
/// one voice regardless of which gate declined.
fn parse_home_table(table: &str) -> ApiResult<AnchorTable> {
    match table {
        "kb_contexts" => Ok(AnchorTable::Contexts),
        "kb_cogmaps" => Ok(AnchorTable::Cogmaps),
        other => Err(ApiError::BadRequest(format!(
            "blob_commit: a blob needs a home (a kb_contexts or kb_cogmaps anchor) — got home \
             table {other}"
        ))),
    }
}

/// Begin a segmented upload — declare the home and media type, get the session id
///
/// A staged session is not a blob: it has no hash yet, it appears in no list, no graph
/// walk, no read surface — only its owner can append to it, read its progress, or finalize
/// it. Home standing is checked here (fail fast) AND at finalize (authoritative — standing
/// can change mid-upload); the allowlist is not consulted at all until the wrapper sees
/// the commit. Same disabled refusal as the single-request path: a session begun on an
/// unconfigured instance could never finalize.
#[utoipa::path(
    post,
    operation_id = "begin_blob_upload",
    path = "/api/blobs/uploads",
    tag = "Blobs",
    security(("bearer_auth" = [])),
    request_body = BlobUploadBeginRequest,
    responses(
        (status = 200, description = "Upload session created", body = BlobUploadBeginResponse),
        (status = 400, description = "Refused — unknown home anchor table, or the instance has no blob store configured", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Home not found or not readable — indistinguishable by design", body = ErrorBody),
        (status = 403, description = "Home readable but not authorable", body = ErrorBody),
    )
)]
pub async fn begin_upload(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<BlobUploadBeginRequest>,
) -> ApiResult<Json<BlobUploadBeginResponse>> {
    if state.blob_store.is_none() || state.config.blob.is_none() {
        return Err(blob_disabled());
    }
    let caller = ProfileId::from(auth.0.profile().id);
    let home = AnchorRef {
        table: parse_home_table(&payload.home_table)?,
        id: payload.home_id,
    };
    let upload_id = temper_services::services::blob_service::begin_upload(
        &state.pool,
        caller,
        home,
        payload.content_type,
    )
    .await?;
    Ok(Json(BlobUploadBeginResponse { upload_id }))
}

#[derive(Debug, Deserialize)]
pub struct SegmentQuery {
    pub seq: u32,
}

/// Append one segment to a staged upload — raw bytes as the request body
///
/// The segment's sha256 rides `x-segment-sha256` (bare hex, the idempotent-append
/// identity): re-sending the same segment at the same seq is a no-op; a DIFFERENT segment
/// at an occupied seq is a 409 — occupied seqs are never superseded. The staging ceiling
/// (`BlobConfig::max_bytes`, the cumulative bound across appends) is enforced in the
/// service; the per-request body bound is the platform's, raised for this door only.
#[utoipa::path(
    post,
    operation_id = "append_blob_segment",
    path = "/api/blobs/uploads/{id}/segments",
    tag = "Blobs",
    params(
        ("id" = Uuid, Path, description = "Upload session ID"),
        ("seq" = u32, Query, description = "Segment ordinal — the seq order is the assembly order at finalize"),
    ),
    security(("bearer_auth" = [])),
    request_body(content_type = "application/octet-stream", description = "The segment's raw bytes"),
    responses(
        (status = 200, description = "Segment landed (or already landed — idempotent); the currently-landed set returned", body = BlobUploadProgress),
        (status = 400, description = "Refused — staging ceiling, missing or malformed x-segment-sha256", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Session absent or not the caller's — indistinguishable by design", body = ErrorBody),
        (status = 409, description = "A different segment occupies this seq", body = ErrorBody),
    )
)]
pub async fn append_segment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(upload_id): Path<Uuid>,
    Query(q): Query<SegmentQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<BlobUploadProgress>> {
    let config = state.config.blob.clone().ok_or_else(blob_disabled)?;
    let segment_hash = headers
        .get("x-segment-sha256")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            ApiError::BadRequest(
                "x-segment-sha256 is required — the segment's sha256 is the idempotent-append \
                 identity"
                    .to_string(),
            )
        })?;
    if segment_hash.len() != 64 {
        return Err(ApiError::BadRequest(format!(
            "x-segment-sha256 must be bare sha256 hex (64 chars) — got {} chars",
            segment_hash.len()
        )));
    }
    let caller = ProfileId::from(auth.0.profile().id);
    let progress = temper_services::services::blob_service::append_to_upload(
        &state.pool,
        &config,
        caller,
        upload_id,
        q.seq,
        body,
    )
    .await?;
    Ok(Json(progress))
}

/// Read a staged upload's progress — the resume read
///
/// Returns the currently-landed segment set and the running byte total: the values a
/// finalize echoes back as its concurrency tokens. The staging is caller-private until
/// finalized — another profile's session answers 404, the same absent-not-refused posture
/// every visibility gate renders.
#[utoipa::path(
    get,
    operation_id = "blob_upload_progress",
    path = "/api/blobs/uploads/{id}",
    tag = "Blobs",
    params(("id" = Uuid, Path, description = "Upload session ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The currently-landed segment set", body = BlobUploadProgress),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Session absent or not the caller's — indistinguishable by design", body = ErrorBody),
    )
)]
pub async fn upload_progress(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(upload_id): Path<Uuid>,
) -> ApiResult<Json<BlobUploadProgress>> {
    // No disabled gate here: begin refuses an unconfigured instance, so no session can
    // exist for this handler to find — a 404 is the only honest answer either way.
    let caller = ProfileId::from(auth.0.profile().id);
    let progress =
        temper_services::services::blob_service::upload_progress(&state.pool, caller, upload_id)
            .await?;
    Ok(Json(progress))
}

/// Finalize a staged upload — assemble, hash, commit
///
/// Assembles the staged segments in seq order and runs the exact single-request commit
/// path: standing re-run (the gate the put answers to), concurrency tokens checked (409,
/// resumable), optional integrity hash checked (422 — the ingest precedent's face), the
/// readability-gated dedup pre-check, the provider put unless deduped, then the SQL
/// wrapper whose cap/allowlist refusals surface verbatim. Staging dies on success only —
/// every failure keeps it, resumable.
#[utoipa::path(
    post,
    operation_id = "finalize_blob_upload",
    path = "/api/blobs/uploads/{id}/finalize",
    tag = "Blobs",
    params(("id" = Uuid, Path, description = "Upload session ID")),
    security(("bearer_auth" = [])),
    request_body = BlobUploadFinalizeRequest,
    responses(
        (status = 200, description = "Committed (or dedup-hit) blob — the same shape the single-request path returns", body = BlobCommitResponse),
        (status = 400, description = "Refused — the wrapper's cap/allowlist vocabulary, verbatim", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Session absent or not the caller's — indistinguishable by design", body = ErrorBody),
        (status = 409, description = "Concurrency tokens stale — segments landed since the caller's last append (staging kept, resumable)", body = ErrorBody),
        (status = 422, description = "Assembled bytes do not hash to the declared expected_content_hash (staging kept)", body = ErrorBody),
    )
)]
pub async fn finalize_upload(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(upload_id): Path<Uuid>,
    Json(payload): Json<BlobUploadFinalizeRequest>,
) -> ApiResult<Json<BlobCommitResponse>> {
    let store = state.blob_store.as_deref().ok_or_else(blob_disabled)?;
    let config = state.config.blob.clone().ok_or_else(blob_disabled)?;
    let caller = ProfileId::from(auth.0.profile().id);
    let outcome = temper_services::services::blob_service::finalize_upload(
        &state.pool,
        store,
        &config,
        caller,
        upload_id,
        &payload,
    )
    .await?;
    Ok(Json(BlobCommitResponse {
        blob_id: outcome.blob_id,
        content_hash: outcome.content_hash,
        content_type: outcome.content_type,
        content_bytes: outcome.content_bytes,
        deduped: outcome.deduped,
    }))
}
