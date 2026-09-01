//! Blob commit + read-through (spec: binary blobs, 2026-09-01 — D6/D7/D9; task
//! `01a05d01-6da8-7272-907f-9b01ba3239c0`, segment S2).
//!
//! **The enforcing authority is the SQL wrapper** (`blob_commit`): cap and allowlist refusals
//! are raised there, from the very config values passed through
//! [`temper_substrate::writes::CommitBlobParams`], and this service surfaces those messages
//! verbatim as `BadRequest` — one vocabulary, taught from the values the operator set, never
//! restated Rust-side (D9). The one Rust-side gate that is NOT duplication is the D7
//! single-request threshold: it is a request-shape question (how large a body this endpoint
//! accepts), decided before any byte reaches the provider, and the refusal names the
//! vocabulary — the threshold and the segmented path beyond it.
//!
//! **Visibility is never decided here.** Reads gate through `blob_readable_by_profile` (the
//! predicate migration `20260901000020` named for this caller); the commit path's dedup
//! pre-check is readability-gated too, so an invisible first home can never be discovered by
//! committing its bytes a second time.

use bytes::Bytes;
use sqlx::PgPool;
use temper_core::types::blob::{
    BlobUploadFinalizeRequest, BlobUploadProgress, BlobUploadSegmentInfo,
};
use temper_core::types::ids::{BlobId, ProfileId};
use temper_substrate::uploads::LandedSegment;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// The CDN cache window a `put` asks of the provider and the `Cache-Control` the read-through
/// response speaks — ONE number, because both name the same posture: content-addressed bytes
/// are immutable (D1), so the strongest cache posture available is safe (D6). A year is the
/// convention for immutable content.
pub const IMMUTABLE_CACHE_MAX_AGE: u32 = 365 * 24 * 60 * 60;

/// The `Cache-Control` header value the read-through response carries, derived from
/// [`IMMUTABLE_CACHE_MAX_AGE`] so the provider cache window and the client cache window
/// cannot drift.
pub fn immutable_cache_control() -> String {
    format!("public, max-age={IMMUTABLE_CACHE_MAX_AGE}, immutable")
}

/// What a commit reports to its surface. `blob_id` is the id the bytes live under — freshly
/// minted, or the EXISTING id on a dedup hit (first home stands, D2). `deduped` reports a
/// readability-gated hit: the caller can already read a blob holding these bytes, so the
/// provider upload was skipped.
pub struct BlobCommitOutcome {
    pub blob_id: BlobId,
    pub content_hash: String,
    pub deduped: bool,
}

/// The wire's home anchor restricts to the two kinds a blob can be homed in (D2). Anything
/// else is refused in the wrapper's own terms.
fn home_gate_tables(
    home: &temper_substrate::payloads::AnchorRef,
) -> ApiResult<(&'static str, &'static str)> {
    match home.table {
        temper_substrate::payloads::AnchorTable::Contexts => {
            Ok(("kb_contexts", "context_authorable_by_profile"))
        }
        temper_substrate::payloads::AnchorTable::Cogmaps => {
            Ok(("kb_cogmaps", "cogmap_authorable_by_profile"))
        }
        _ => Err(ApiError::BadRequest(
            "blob_commit: a blob needs a home (a kb_contexts or kb_cogmaps anchor)".to_string(),
        )),
    }
}

/// Auth before writes — the placement gate, mirroring the incumbent two-step exactly:
///
/// 1. **Read gate first** (`anchor_readable_by_profile`): a home the caller cannot read is
///    `NotFound` — the same absent-not-refused posture every visibility gate renders, so a
///    probe over context/cogmap ids learns nothing.
/// 2. **Then the authority gate** (`context_authorable_by_profile` /
///    `cogmap_authorable_by_profile`, the `check_context_authorable` placement precedent):
///    readable-but-not-writable is `Forbidden` (403). Reaching this arm means the read gate
///    already passed, so the anchor's existence is not news — the same leak analysis that
///    closed audit finding F-2.
///
/// Both gates run BEFORE the provider put, so an under-scoped commit costs no provider work
/// and leaves no orphan bytes at the content-addressed pathname.
async fn check_home_standing(
    pool: &PgPool,
    caller: ProfileId,
    home: &temper_substrate::payloads::AnchorRef,
) -> ApiResult<()> {
    let (table, authorable_fn) = home_gate_tables(home)?;

    let readable: bool = sqlx::query_scalar!(
        r#"SELECT anchor_readable_by_profile($1, $2, $3) AS "readable!""#,
        caller.uuid(),
        table,
        home.id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !readable {
        return Err(ApiError::NotFound("home not found".to_string()));
    }

    let authorable: Option<bool> = match authorable_fn {
        "context_authorable_by_profile" => sqlx::query_scalar!(
            "SELECT context_authorable_by_profile($1, $2)",
            caller.uuid(),
            home.id,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?,
        _ => sqlx::query_scalar!(
            "SELECT cogmap_authorable_by_profile($1, $2)",
            caller.uuid(),
            home.id,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?,
    };
    if !authorable.unwrap_or(false) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// Commit bytes as a blob: dedup pre-check, provider put at the content-addressed pathname,
/// then the substrate's attributed write (`commit_blob_with` — provider presence verified
/// before the ledger ever sees the event, D4). The caller acts as themselves: owner is the
/// authenticated profile, the emitter is their `web` surface entity. Home standing is gated
/// before any of it (auth before writes).
pub async fn commit_blob(
    pool: &PgPool,
    store: &dyn temper_substrate::blob_store::BlobStore,
    config: &crate::config::BlobConfig,
    caller: ProfileId,
    home: temper_substrate::payloads::AnchorRef,
    content_type: String,
    bytes: Bytes,
) -> ApiResult<BlobCommitOutcome> {
    check_home_standing(pool, caller, &home).await?;

    let content_hash = temper_core::hash::sha256_hex(&bytes);
    let pathname = temper_substrate::blob_store::blob_pathname(&content_hash);

    let deduped = temper_substrate::readback::readable_blob_id_by_hash(pool, caller, &content_hash)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .is_some();
    if !deduped {
        store
            .put(
                &pathname,
                &content_type,
                bytes.clone(),
                IMMUTABLE_CACHE_MAX_AGE,
            )
            .await
            .map_err(|e| ApiError::Internal(format!("blob provider put failed: {e}")))?;
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let id = temper_substrate::writes::commit_blob(
        pool,
        store,
        temper_substrate::writes::CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home,
            owner: caller,
            originator: None,
            content_hash: content_hash.clone(),
            content_type,
            content_bytes: bytes.len() as i64,
            max_bytes: config.max_bytes,
            allowlist: &config.allowlist,
            emitter,
        },
    )
    .await
    .map_err(map_commit_err)?;

    Ok(BlobCommitOutcome {
        blob_id: id,
        content_hash,
        deduped,
    })
}

/// Stream a visible blob's bytes through the provider (D6 read-through). Visibility is the
/// row gate (`blob_by_id`); the provider is asked only afterwards — an invisible blob costs
/// no provider round trip and leaves no existence trace. `consistent=false`: the bytes are
/// immutable, so the provider CDN cache is always telling the truth (the consistent flag is
/// the erasure task's escape hatch, not the read path's).
pub async fn read_through(
    pool: &PgPool,
    store: &dyn temper_substrate::blob_store::BlobStore,
    caller: ProfileId,
    blob: BlobId,
) -> ApiResult<(
    temper_substrate::readback::RetrievedBlob,
    temper_substrate::blob_store::ByteStream,
)> {
    let row = temper_substrate::readback::blob_by_id(pool, caller, blob)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("blob not found".to_string()))?;

    let stream = store
        .get(&row.blob_pathname, false)
        .await
        .map_err(|e| ApiError::Internal(format!("blob provider read failed: {e}")))?;

    Ok((row, stream))
}

/// Map a substrate write error. The SQL wrapper's refusals RAISE with a `blob_commit:` prefix
/// and carry the D9 vocabulary verbatim (cap, allowlist, home, addressing) — those are the
/// caller's own state, safe and required to surface as `400` (the `finalize_err` precedent of
/// walking the `anyhow` chain to the sqlx error under it). Anything else is a 500.
fn map_commit_err(e: anyhow::Error) -> ApiError {
    for cause in e.chain() {
        if let Some(sqlx::Error::Database(db)) = cause.downcast_ref::<sqlx::Error>() {
            if db.message().starts_with("blob_commit:") {
                return ApiError::BadRequest(db.message().to_string());
            }
        }
    }
    ApiError::Internal(format!("blob commit failed: {e}"))
}

// ── Segmented upload (S3, D7) ─────────────────────────────────────────────────────
// The segmented-INGEST precedent (begin/append/finalize over the same content-addressed
// target), in blob terms. The staging rows are pre-ledger transport state owned by
// `temper_substrate::uploads` — their gate is owner-equality on the session row, never
// `blob_readable_by_profile` (a staged session is not a blob; it has no hash yet). What
// this service adds around those rows is exactly what it adds around the single-request
// commit: the F-2 standing two-step, the readability-gated dedup pre-check, the provider
// put, and the wrapper's verbatim refusals — same ordering, same authorities.

/// What a finalize reports to its surface: the commit outcome plus the two fields the
/// wire response needs that live on the session (the media type it declared at begin,
/// and the assembled whole's byte count).
pub struct BlobUploadFinalizeOutcome {
    pub blob_id: BlobId,
    pub content_hash: String,
    pub deduped: bool,
    pub content_type: String,
    pub content_bytes: i64,
}

/// Begin a staged upload: standing two-step on the declared home (fail fast — no orphan
/// session for the unauthorized), then the server-minted session row. The allowlist is
/// NOT examined here: the SQL wrapper is the sole allowlist authority, at finalize (D9).
pub async fn begin_upload(
    pool: &PgPool,
    caller: ProfileId,
    home: temper_substrate::payloads::AnchorRef,
    content_type: String,
) -> ApiResult<Uuid> {
    check_home_standing(pool, caller, &home).await?;
    temper_substrate::uploads::create_session(pool, caller, &home, &content_type)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload begin failed: {e}")))
}

/// The landed set as the wire sees it. `None` from the substrate means the session is
/// absent OR not the caller's — the same 404 either way (a probe over upload ids learns
/// nothing).
fn progress_from(upload_id: Uuid, landed: Vec<LandedSegment>) -> BlobUploadProgress {
    let total_bytes = landed.iter().map(|s| s.segment_bytes).sum();
    BlobUploadProgress {
        upload_id,
        segments: landed
            .into_iter()
            .map(|s| BlobUploadSegmentInfo {
                seq: s.seq as u32,
                segment_hash: s.segment_hash,
                segment_bytes: s.segment_bytes,
            })
            .collect(),
        total_bytes,
    }
}

/// Append one segment. Owner gate via the substrate's reads (absent-or-not-yours ⇒ 404),
/// then the staging bound: staged total plus this segment may not exceed
/// `BlobConfig::max_bytes`. The bound is the staging ceiling — how many bytes this
/// server will hold between begin and finalize, the D7-threshold's segmented twin — and
/// it is what makes finalize's put-before-commit safe: an assembled whole over the cap
/// can never exist to reach the provider. The commit-time cap itself stays the SQL
/// wrapper's authority, enforced at finalize.
pub async fn append_to_upload(
    pool: &PgPool,
    config: &crate::config::BlobConfig,
    caller: ProfileId,
    upload_id: Uuid,
    seq: u32,
    bytes: Bytes,
) -> ApiResult<BlobUploadProgress> {
    let landed = temper_substrate::uploads::landed_segments(pool, caller, upload_id)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload read failed: {e}")))?
        .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;
    let staged: i64 = landed.iter().map(|s| s.segment_bytes).sum();
    let incoming = bytes.len() as i64;
    if staged + incoming > config.max_bytes {
        return Err(ApiError::BadRequest(format!(
            "blob_upload: this append would put staged bytes at {} against a staging ceiling \
             of {} — an upload stages at most one blob's worth of bytes; the cap the commit \
             enforces at finalize is the same ceiling",
            staged + incoming,
            config.max_bytes
        )));
    }

    let segment_hash = temper_core::hash::sha256_hex(&bytes);
    let outcome = temper_substrate::uploads::append_segment(
        pool,
        caller,
        upload_id,
        seq as i32,
        &bytes,
        &segment_hash,
    )
    .await
    .map_err(|e| ApiError::Internal(format!("blob upload append failed: {e}")))?;
    match outcome {
        None => Err(ApiError::NotFound("upload not found".to_string())),
        Some(temper_substrate::uploads::AppendOutcome::Conflict { existing_hash }) => {
            Err(ApiError::Conflict(format!(
                "segment seq {seq} already landed with hash {existing_hash} — occupied seqs \
                 are never superseded; the assembled whole must stay unambiguous"
            )))
        }
        Some(_) => {
            let landed = temper_substrate::uploads::landed_segments(pool, caller, upload_id)
                .await
                .map_err(|e| ApiError::Internal(format!("blob upload read failed: {e}")))?
                .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;
            Ok(progress_from(upload_id, landed))
        }
    }
}

/// The resume/progress read: the currently-landed set plus the running byte total — the
/// server-handed values a finalize echoes back (`expected_segments`, `expected_total_bytes`).
pub async fn upload_progress(
    pool: &PgPool,
    caller: ProfileId,
    upload_id: Uuid,
) -> ApiResult<BlobUploadProgress> {
    let landed = temper_substrate::uploads::landed_segments(pool, caller, upload_id)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload read failed: {e}")))?
        .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;
    Ok(progress_from(upload_id, landed))
}

/// Finalize a staged upload: assemble in seq order, hash, then exactly the S2 commit
/// path — standing re-run (authoritative: standing can change mid-upload, and it gates
/// the put), concurrency tokens checked (a mismatch is [`ApiError::Conflict`] —
/// resumable, staging kept), optional integrity hash checked (a mismatch is
/// [`ApiError::ContentIntegrity`] — the ingest precedent's face for "the assembled bytes
/// do not hash to the declaration"), readability-gated dedup pre-check, provider put
/// unless deduped, then `commit_blob` whose cap/allowlist refusals surface verbatim.
/// Staging dies on success only; every failure keeps it (keep-and-declare — a TTL
/// reaper is a declared hole, never silently clean).
pub async fn finalize_upload(
    pool: &PgPool,
    store: &dyn temper_substrate::blob_store::BlobStore,
    config: &crate::config::BlobConfig,
    caller: ProfileId,
    upload_id: Uuid,
    req: &BlobUploadFinalizeRequest,
) -> ApiResult<BlobUploadFinalizeOutcome> {
    let session = temper_substrate::uploads::load_session(pool, caller, upload_id)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload read failed: {e}")))?
        .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;

    // Auth before writes, again: the begin-time standing was a fail-fast courtesy; this
    // is the gate the put answers to.
    check_home_standing(pool, caller, &session.home).await?;

    let landed = temper_substrate::uploads::landed_segments(pool, caller, upload_id)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload read failed: {e}")))?
        .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;
    let total_bytes: i64 = landed.iter().map(|s| s.segment_bytes).sum();
    // Concurrency tokens — "nothing landed since my last append". A mismatch leaves the
    // staging exactly as it is: resumable (re-read progress, re-finalize).
    if landed.len() as u32 != req.expected_segments || total_bytes != req.expected_total_bytes {
        return Err(ApiError::Conflict(format!(
            "staged state is {} segments / {} bytes against expected {} / {} — landed since \
             your last append; re-read the progress and re-finalize",
            landed.len(),
            total_bytes,
            req.expected_segments,
            req.expected_total_bytes
        )));
    }

    let body = temper_substrate::uploads::assemble_body(pool, upload_id)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload assemble failed: {e}")))?;
    let content_hash = temper_core::hash::sha256_hex(&body);
    if let Some(expected) = &req.expected_content_hash {
        if expected != &content_hash {
            // The ingest precedent's ContentIntegrity face: the stored bytes do not hash
            // to the declaration, and an occupied seq is never superseded — the caller
            // begins a new session rather than patching this one.
            return Err(ApiError::ContentIntegrity(format!(
                "the assembled bytes hash to {content_hash}, not the declared {expected} — \
                 staged seqs are never superseded; begin a new upload"
            )));
        }
    }

    let deduped = temper_substrate::readback::readable_blob_id_by_hash(pool, caller, &content_hash)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .is_some();
    if !deduped {
        store
            .put(
                &temper_substrate::blob_store::blob_pathname(&content_hash),
                &session.content_type,
                body.clone().into(),
                IMMUTABLE_CACHE_MAX_AGE,
            )
            .await
            .map_err(|e| ApiError::Internal(format!("blob provider put failed: {e}")))?;
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let blob_id = temper_substrate::writes::commit_blob(
        pool,
        store,
        temper_substrate::writes::CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: session.home,
            owner: caller,
            originator: None,
            content_hash: content_hash.clone(),
            content_type: session.content_type.clone(),
            content_bytes: body.len() as i64,
            max_bytes: config.max_bytes,
            allowlist: &config.allowlist,
            emitter,
        },
    )
    .await
    .map_err(map_commit_err)?;

    // Success only. A refusal above leaves the staging in place — resumable, and honest
    // about it (the TTL reaper is the declared hole, not a silent sweep).
    temper_substrate::uploads::delete_session(pool, upload_id)
        .await
        .map_err(|e| ApiError::Internal(format!("blob upload cleanup failed: {e}")))?;

    Ok(BlobUploadFinalizeOutcome {
        blob_id,
        content_hash,
        deduped,
        content_type: session.content_type,
        content_bytes: body.len() as i64,
    })
}

// ── Blob list + relations (S4) ────────────────────────────────────────────────────
// The relate surface is the D3 commitment made addressable: relations are ordinary
// `kb_edges` rows (the endpoint CHECK admitted `kb_blobs` at S1), so asserting one is the
// substrate's ordinary relationship write — what this service adds is the blob-scoped
// gate train and the home resolution the generic write deliberately does not do. The
// list/relations reads gate through the NAMED predicates (`blob_readable_by_profile`,
// `edges_visible_to`) — never a restatement, so the two doors onto one anchor cannot
// disagree about which blobs or which edges exist.

use temper_core::types::blob::{
    BlobRelationAck as WireRelationAck, BlobRelationAssertRequest, BlobRelationDirection,
    BlobRelationRow as WireRelationRow, BlobSummary as WireBlobSummary,
};
use temper_core::types::graph::{EdgeKind as WireEdgeKind, Polarity as WirePolarity};

/// The blob's home anchor, readable-gated: `None` means the blob is absent OR not the
/// caller's — indistinguishable, the 404 either way (a probe over blob ids learns
/// nothing). This is the existence gate and the home read in ONE query, so the gate cannot
/// pass while the home read fails — the S3 `landed_segments` lesson applied at the shape
/// level: the gate is the row fetch, never a filter emptied afterwards.
async fn blob_home(
    pool: &PgPool,
    caller: ProfileId,
    blob: BlobId,
) -> ApiResult<Option<(temper_substrate::payloads::AnchorTable, uuid::Uuid)>> {
    let row = sqlx::query!(
        r#"SELECT h.anchor_table, h.anchor_id
             FROM kb_blob_homes h
            WHERE h.blob_id = $1
              AND blob_readable_by_profile($2, $1)"#,
        blob.uuid(),
        caller.uuid(),
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(row.map(|r| {
        let table = match r.anchor_table.as_str() {
            "kb_contexts" => temper_substrate::payloads::AnchorTable::Contexts,
            _ => temper_substrate::payloads::AnchorTable::Cogmaps,
        };
        (table, r.anchor_id)
    }))
}

/// Container-write gate on an anchor of either kind — the `check_container_authorable`
/// rule in service terms: an edge is an object HOMED in the anchor, so authoring one is
/// authoring into that container. Reachable only after the read gate passed, so the 403
/// discloses nothing an earlier read did not (the F-2 leak analysis).
async fn check_home_authorable(
    pool: &PgPool,
    caller: ProfileId,
    table: temper_substrate::payloads::AnchorTable,
    anchor_id: uuid::Uuid,
) -> ApiResult<()> {
    let authorable: Option<bool> = match table {
        temper_substrate::payloads::AnchorTable::Contexts => sqlx::query_scalar!(
            "SELECT context_authorable_by_profile($1, $2)",
            caller.uuid(),
            anchor_id,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?,
        _ => sqlx::query_scalar!(
            "SELECT cogmap_authorable_by_profile($1, $2)",
            caller.uuid(),
            anchor_id,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?,
    };
    if !authorable.unwrap_or(false) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// The peer-endpoint gate: you may point at what you can see (`endpoint_readable_by_profile`,
/// the same predicate the incumbent edge writes apply — its `kb_blobs` arm landed with the S1
/// reads migration). Invisible-or-absent renders `NotFound`: the write must not become an
/// existence oracle over anchors the caller cannot read.
async fn check_peer_readable(
    pool: &PgPool,
    caller: ProfileId,
    peer: &temper_substrate::payloads::AnchorRef,
) -> ApiResult<()> {
    let readable: Option<bool> = sqlx::query_scalar!(
        "SELECT endpoint_readable_by_profile($1, $2, $3)",
        caller.uuid(),
        peer.table.as_str(),
        peer.id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !readable.unwrap_or(false) {
        return Err(ApiError::NotFound(
            "relation peer not found or not readable".to_string(),
        ));
    }
    Ok(())
}

/// Assert one relation between a blob and a peer anchor. Gate train, in order (auth before
/// writes): blob readable → 404; home authorable → 403; peer readable → 404. Then the
/// substrate's ordinary relationship write, homed on the BLOB's home — the blob-scoped
/// surface answers to the blob's standing, and the edge is therefore readable by exactly
/// the blob-home's readers, which is the visibility story D3's negative face rides on.
///
/// Idempotent by the projector's active-edge invariant: re-asserting the same
/// (source, target, kind, label, home) updates the weight and returns the SAME edge — a
/// relation neither created nor removed any other (`one-blob-many-relations`).
///
/// Refusals speak the `blob_relate:` vocabulary for the parts this surface owns (the peer
/// table parse lives handler-side, in the same one-parse-point terms as `parse_home`);
/// everything downstream is the substrate's own voice.
pub async fn relate_blob(
    pool: &PgPool,
    caller: ProfileId,
    blob: BlobId,
    req: &BlobRelationAssertRequest,
    act: temper_core::types::authorship::ActContext,
) -> ApiResult<WireRelationAck> {
    let (home_table, home_id) = blob_home(pool, caller, blob)
        .await?
        .ok_or_else(|| ApiError::NotFound("blob not found".to_string()))?;
    check_home_authorable(pool, caller, home_table, home_id).await?;

    // The peer-table parse lives here rather than the handler so the vocabulary and the
    // AnchorRef are built in one place (the `parse_home` rule): the refusal mirrors the
    // substrate's endpoint terms, so the caller hears one voice regardless of which gate
    // declined.
    use temper_substrate::payloads::AnchorTable as T;
    let peer_table = match req.peer_table.as_str() {
        "kb_resources" => T::Resources,
        "kb_cogmaps" => T::Cogmaps,
        "kb_blobs" => T::Blobs,
        other => {
            return Err(ApiError::BadRequest(format!(
                "blob_relate: a relation points at a kb_resources, kb_cogmaps or kb_blobs \
                 anchor — got peer table {other}"
            )))
        }
    };
    let peer = temper_substrate::payloads::AnchorRef {
        table: peer_table,
        id: req.peer_id,
    };
    check_peer_readable(pool, caller, &peer).await?;

    let home = match home_table {
        temper_substrate::payloads::AnchorTable::Contexts => {
            temper_substrate::events::EdgeHome::Context(temper_core::types::ids::ContextId::from(
                home_id,
            ))
        }
        _ => temper_substrate::events::EdgeHome::Cogmap(temper_core::types::ids::CogmapId::from(
            home_id,
        )),
    };
    let (source, target) = match req.direction {
        BlobRelationDirection::BlobAsSource => {
            (temper_substrate::payloads::AnchorRef::blob(blob), peer)
        }
        BlobRelationDirection::BlobAsTarget => {
            (peer, temper_substrate::payloads::AnchorRef::blob(blob))
        }
    };
    let kind = match req.edge_kind {
        WireEdgeKind::Express => temper_substrate::affinity::EdgeKind::Express,
        WireEdgeKind::Contains => temper_substrate::affinity::EdgeKind::Contains,
        WireEdgeKind::LeadsTo => temper_substrate::affinity::EdgeKind::LeadsTo,
        WireEdgeKind::Near => temper_substrate::affinity::EdgeKind::Near,
    };
    let polarity = match req.polarity {
        WirePolarity::Forward => temper_substrate::payloads::EdgePolarity::Forward,
        WirePolarity::Inverse => temper_substrate::payloads::EdgePolarity::Inverse,
    };
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let label = (!req.label.is_empty()).then_some(req.label.as_str());

    let edge = temper_substrate::writes::assert_anchored_edge_with(
        pool,
        temper_substrate::writes::AssertAnchoredEdgeParams {
            source,
            target,
            kind,
            polarity,
            label,
            weight: req.weight,
            home,
            emitter,
        },
        temper_substrate::events::EventContext {
            authorship: act.authorship,
            invocation: act.invocation,
            correlation: act.correlation,
        },
    )
    .await
    .map_err(|e| ApiError::Internal(format!("blob relation assert failed: {e}")))?;

    Ok(WireRelationAck {
        edge_handle: edge.uuid(),
    })
}

/// List the blobs the caller can read, optionally scoped to one home anchor. The gate is
/// the substrate's set read — `blob_readable_by_profile`, the NAMED predicate — so this
/// surface honors visibility and cannot redefine it (the register's list-surfaces arm).
pub async fn list_blobs(
    pool: &PgPool,
    caller: ProfileId,
    home: Option<temper_substrate::payloads::AnchorRef>,
) -> ApiResult<Vec<WireBlobSummary>> {
    let rows = temper_substrate::readback::blobs_readable_by_profile(pool, caller, home.as_ref())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|r| WireBlobSummary {
            blob_id: r.blob_id,
            content_hash: r.content_hash,
            content_type: r.content_type,
            content_bytes: r.content_bytes,
            created: r.created,
        })
        .collect())
}

/// The edges incident to one blob, visible under the caller's standing — the dedicated
/// read surface D3 promises ("what relates to this blob"). An invisible-or-absent blob is
/// `NotFound` (the read-through's face); a visible blob with no relations is an empty list.
pub async fn blob_relations(
    pool: &PgPool,
    caller: ProfileId,
    blob: BlobId,
) -> ApiResult<Vec<WireRelationRow>> {
    let rows = temper_substrate::readback::blob_relations(pool, caller, blob)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("blob not found".to_string()))?;

    rows.into_iter()
        .map(|r| {
            Ok(WireRelationRow {
                edge_id: r.edge_id.uuid(),
                peer_table: r.peer_table,
                peer_id: r.peer_id,
                peer_title: r.peer_title,
                edge_kind: parse_wire_edge_kind(&r.edge_kind)?,
                polarity: parse_wire_polarity(&r.polarity)?,
                label: r.label,
                direction: r.direction,
                weight: r.weight,
                created: r.created,
            })
        })
        .collect()
}

/// The SQL enum's value set is fixed by DDL; a miss means the DB and the code disagree,
/// which is a 500, never a caller-facing refusal.
fn parse_wire_edge_kind(s: &str) -> ApiResult<WireEdgeKind> {
    match s {
        "express" => Ok(WireEdgeKind::Express),
        "contains" => Ok(WireEdgeKind::Contains),
        "leads_to" => Ok(WireEdgeKind::LeadsTo),
        "near" => Ok(WireEdgeKind::Near),
        other => Err(ApiError::Internal(format!(
            "unknown edge_kind value in kb_edges: {other}"
        ))),
    }
}

fn parse_wire_polarity(s: &str) -> ApiResult<WirePolarity> {
    match s {
        "forward" => Ok(WirePolarity::Forward),
        "inverse" => Ok(WirePolarity::Inverse),
        other => Err(ApiError::Internal(format!(
            "unknown edge_polarity value in kb_edges: {other}"
        ))),
    }
}
