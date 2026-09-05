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
//! predicate migration named for this caller). Identity is PER-HOME (D2 as amended
//! 2026-09-02): the commit's dedup pre-check asks only about the caller's OWN home, so the
//! same bytes in a scope the caller cannot see are invisible and commit as the caller's own
//! fresh row — a principal's record is asserted by their own event, never another
//! principal's identity.

use bytes::Bytes;
use sqlx::PgPool;
use temper_core::types::blob::{
    BlobUploadFinalizeRequest, BlobUploadProgress, BlobUploadSegmentInfo,
};
use temper_core::types::ids::{BlobId, ProfileId};
use temper_substrate::uploads::LandedSegment;
use temper_workflow::operations::Surface;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// The CDN cache window a `put` asks of the provider and the `Cache-Control` the read-through
/// response speaks — ONE number, because both name the same posture: content-addressed bytes
/// are immutable (D1), so a long window is safe (D6). A year is the convention for immutable
/// content.
pub const IMMUTABLE_CACHE_MAX_AGE: u32 = 365 * 24 * 60 * 60;

/// The `Cache-Control` header value the read-through response carries, derived from
/// [`IMMUTABLE_CACHE_MAX_AGE`] so the provider cache window and the client cache window
/// cannot drift. The response is PER-CALLER AUTHORIZED bytes, so `private`: a shared cache
/// (CDN, corporate proxy, platform edge) is never licensed to store it and serve one
/// principal's bytes to another. `immutable` stays — content addressing, not caching policy,
/// is what earns it (D1).
pub fn immutable_cache_control() -> String {
    format!("private, max-age={IMMUTABLE_CACHE_MAX_AGE}, immutable")
}

/// The `Content-Disposition` every read-through response carries (F10 ruling,
/// 2026-09-02): `attachment`, unconditionally. A blob read is a bytes fetch, never a
/// rendering invitation — and the posture must survive the operational changes the
/// review named: a cookie-auth flip, a CSP relaxation, an allowlist edit (the allowlist
/// is commit-time only, so removing a type never stops serving committed bytes).
/// Subresource rendering (`<img>`, `<video>`, `<audio>`) ignores the header, so
/// legitimate embedding is unaffected; what `attachment` stops is NAVIGATION — the
/// browser downloads instead of rendering, so stored active content (SVG included)
/// cannot execute in the app's origin.
pub const BLOB_CONTENT_DISPOSITION: &str = "attachment";

/// The D7 single-request threshold refusal, spelled once and spoken by every committing
/// surface: the HTTP multipart handler aborts mid-stream with it (before an over-threshold
/// body is ever fully buffered), and [`commit_blob`] refuses with it before any byte reaches
/// the provider — the MCP commit tool rides that same service seam, so no committing surface
/// can silently skip the threshold. The refusal names the threshold in force and the
/// segmented path beyond it.
pub fn single_request_threshold_refusal(
    content_bytes: usize,
    config: &crate::config::BlobConfig,
) -> Option<ApiError> {
    (content_bytes > config.single_request_max_bytes).then(|| {
        ApiError::BadRequest(format!(
            "this request carries {content_bytes} bytes against a single-request threshold of \
             {} — beyond it, use the segmented upload path (begin/append/finalize); the \
             per-blob cap in force is {} bytes",
            config.single_request_max_bytes, config.max_bytes
        ))
    })
}

/// What a commit reports to its surface. `blob_id` is the id the bytes live under — freshly
/// minted, or the EXISTING id on a dedup hit within the caller's own home (get-or-create is
/// per-home, D2 as amended; a hash known only to other scopes is the caller's fresh row).
/// `deduped` reports a hit in the caller's home: the provider upload was skipped.
/// `content_type` is the row's STORED media type — the first committer's on a dedup hit
/// (N2, 2026-09-03 review: the projector's conflict arm never updates the row, so echoing
/// the caller's re-commit declaration would report a type that was never stored).
pub struct BlobCommitOutcome {
    pub blob_id: BlobId,
    pub content_hash: String,
    pub content_type: String,
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

/// The home parse for the single-request commit, shared by every committing surface (the
/// API handler's multipart fields, the MCP tool's input strings) so the vocabulary and the
/// `AnchorRef` are built in one place. The refusal mirrors the wrapper's terms, and an
/// absent field is named `<absent>` exactly as the handler-side parse always rendered it.
fn parse_home(
    home_table: Option<String>,
    home_id: Option<String>,
) -> ApiResult<temper_substrate::payloads::AnchorRef> {
    use temper_substrate::payloads::{AnchorRef, AnchorTable};
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

/// The refusal for an unconfigured instance, shared by every blob surface so the sentence
/// is spelled once. Names the two config postures S1 landed — the vocabulary, not a bare
/// "unavailable" — so an operator knows what enables the door.
pub fn blob_disabled() -> ApiError {
    ApiError::BadRequest(
        "blob endpoints are disabled — this instance has no blob store configured; set \
         BLOB_STORE_ID (on Vercel, OIDC-first) or BLOB_READ_WRITE_TOKEN (off Vercel) to enable \
         them"
            .to_string(),
    )
}

/// The refusal for a DELIBERATELY closed instance — `BLOB_ENABLED=false`, or an
/// unrecognized value for it (fail closed, loudly). Distinct vocabulary from
/// [`blob_disabled`]: that sentence invites enabling by naming credentials; this one
/// names the knob, because the credentials are not the thing standing in the door.
/// Chosen per instance by `AppState::blob_refusal`, never picked by hand at a door.
pub fn blob_disabled_by_policy() -> ApiError {
    ApiError::BadRequest(
        "blob endpoints are disabled by configuration — BLOB_ENABLED is set to a value \
         that disables the flow; remove the setting or set BLOB_ENABLED=true to enable them"
            .to_string(),
    )
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
    .await?;
    if !readable {
        return Err(ApiError::NotFound("home not found".to_string()));
    }

    let authorable: Option<bool> = match authorable_fn {
        "context_authorable_by_profile" => {
            sqlx::query_scalar!(
                "SELECT context_authorable_by_profile($1, $2)",
                caller.uuid(),
                home.id,
            )
            .fetch_one(pool)
            .await?
        }
        _ => {
            sqlx::query_scalar!(
                "SELECT cogmap_authorable_by_profile($1, $2)",
                caller.uuid(),
                home.id,
            )
            .fetch_one(pool)
            .await?
        }
    };
    if !authorable.unwrap_or(false) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// What a committing surface passes to [`commit_blob`]: who acts, what the bytes are,
/// where the blob homes, and which surface the act arrived on (the emitter marker — the
/// write is attributed to the caller's `<handle>@<marker>` entity).
pub struct BlobCommitCommand {
    pub caller: ProfileId,
    pub home_table: Option<String>,
    pub home_id: Option<String>,
    pub content_type: String,
    pub bytes: Bytes,
    pub surface: Surface,
}

/// The row's STORED media type, read back from the committed row — the N2 rule in one
/// place: on a dedup hit the row is the FIRST committer's, and a committing surface must
/// report what is stored, never what was just declared. **This is the single query site
/// the erasure build's N3 arm must widen**: the `content_type!` override is sound at HEAD
/// (the wrapper refuses a NULL type before the event) and nothing nulls the column until
/// erasure — when it does, THIS read changes shape, and because it is the only one, the
/// widening cannot be forgotten at a duplicate.
async fn stored_content_type(pool: &PgPool, id: uuid::Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT content_type AS "content_type!" FROM kb_blobs WHERE id = $1"#,
        id
    )
    .fetch_one(pool)
    .await
}

/// Commit bytes as a blob: dedup pre-check, provider put at the content-addressed pathname,
/// then the substrate's attributed write (`commit_blob_with` — provider presence verified
/// before the ledger ever sees the event, D4). The caller acts as themselves: owner is the
/// authenticated profile, the emitter is their entity on the surface the commit arrived on
/// (`surface.marker()` — the API degrades untrusted claims to `web`, so the emitter is always
/// a surface the caller actually reached). Home standing is gated before any of it (auth
/// before writes).
///
/// The home parse lives here rather than in any handler (the `parse_home` rule — the
/// peer-table parse's reasoning): the wire type is an enum-shaped string, and the refusal
/// mirrors the wrapper's terms (a kb_contexts or kb_cogmaps anchor) so every surface hears
/// one vocabulary regardless of which gate declined.
pub async fn commit_blob(
    pool: &PgPool,
    store: &dyn temper_substrate::blob_store::BlobStore,
    config: &crate::config::BlobConfig,
    cmd: BlobCommitCommand,
) -> ApiResult<BlobCommitOutcome> {
    let BlobCommitCommand {
        caller,
        home_table,
        home_id,
        content_type,
        bytes,
        surface,
    } = cmd;
    // The D7 threshold is a request-shape question — how large a body this door accepts —
    // so it is the FIRST decision, before the home parse, before standing, before the
    // provider. A surface that reaches this function cannot skip it.
    if let Some(err) = single_request_threshold_refusal(bytes.len(), config) {
        return Err(err);
    }
    let home = parse_home(home_table, home_id)?;
    check_home_standing(pool, caller, &home).await?;

    let content_hash = temper_core::hash::sha256_hex(&bytes);
    let pathname = temper_substrate::blob_store::blob_pathname(&content_hash);

    // The dedup pre-check is home-scoped (D2 as amended): only the caller's own home answers.
    let deduped = temper_substrate::readback::home_blob_id_by_hash(pool, &home, &content_hash)
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob dedup pre-check failed", e))?
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
            .map_err(|e| ApiError::internal_scrubbed("blob provider put failed", e))?;
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, surface.marker())
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob emitter resolve failed", e))?;

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

    // N2 (2026-09-03 review): the outcome reports the row's STORED media type, read
    // back from the committed row — see `stored_content_type`, the single site both
    // committing doors share.
    let stored_type = stored_content_type(pool, id.uuid())
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob commit read failed", e))?;

    Ok(BlobCommitOutcome {
        blob_id: id,
        content_hash,
        content_type: stored_type,
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
        .map_err(|e| ApiError::internal_scrubbed("blob read failed", e))?
        .ok_or_else(|| ApiError::NotFound("blob not found".to_string()))?;

    let stream = store
        .get(&row.blob_pathname, false)
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob provider read failed", e))?;

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
    ApiError::internal_scrubbed("blob commit failed", e)
}

// ── Segmented upload (S3, D7) ─────────────────────────────────────────────────────
// The segmented-INGEST precedent (begin/append/finalize over the same content-addressed
// target), in blob terms. The staging rows are pre-ledger transport state owned by
// `temper_substrate::uploads` — their gate is owner-equality on the session row, never
// `blob_readable_by_profile` (a staged session is not a blob; it has no hash yet). What
// this service adds around those rows is exactly what it adds around the single-request
// commit: the F-2 standing two-step, the readability-gated dedup pre-check, the provider
// put, and the wrapper's verbatim refusals — same ordering, same authorities.

/// What a finalize reports to its surface: the commit outcome plus the assembled whole's
/// byte count. `content_type` is the row's STORED media type (the first committer's on a
/// dedup hit — the same N2 rule the single-request path answers to), not the session's
/// begin-time declaration, which the row may not carry.
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
        .map_err(|e| ApiError::internal_scrubbed("blob upload begin failed", e))
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

/// Append one segment. Owner gate via the substrate's append (absent-or-not-yours ⇒ 404),
/// then the staging bound: the substrate's append enforces the ceiling ATOMICALLY — owner
/// gate, staged total, ceiling decision, and insert in one transaction under the session
/// row's lock — so concurrent appends from the owning principal cannot together stage over
/// `BlobConfig::max_bytes` (the F4 TOCTOU: N appends all read the same staged total and all
/// landed, and the over-cap whole was assembled and put before the wrapper refused). The
/// ceiling is the staging bound — the D7-threshold's segmented twin — and it is what makes
/// finalize's put-before-commit safe: an assembled whole over the cap can never exist to
/// reach the provider. The commit-time cap itself stays the SQL wrapper's authority,
/// enforced at finalize.
pub async fn append_to_upload(
    pool: &PgPool,
    config: &crate::config::BlobConfig,
    caller: ProfileId,
    upload_id: Uuid,
    seq: u32,
    bytes: Bytes,
) -> ApiResult<BlobUploadProgress> {
    let segment_hash = temper_core::hash::sha256_hex(&bytes);
    let outcome = temper_substrate::uploads::append_segment(
        pool,
        caller,
        upload_id,
        seq as i32,
        &bytes,
        &segment_hash,
        config.max_bytes,
    )
    .await
    .map_err(|e| ApiError::internal_scrubbed("blob upload append failed", e))?;
    match outcome {
        None => Err(ApiError::NotFound("upload not found".to_string())),
        Some(temper_substrate::uploads::AppendOutcome::OverCeiling { staged, ceiling }) => {
            Err(ApiError::BadRequest(format!(
                "blob_upload: this append would put staged bytes at {staged} against a \
                 staging ceiling of {ceiling} — an upload stages at most one blob's worth \
                 of bytes; the cap the commit enforces at finalize is the same ceiling"
            )))
        }
        Some(temper_substrate::uploads::AppendOutcome::Conflict { existing_hash }) => {
            Err(ApiError::Conflict(format!(
                "segment seq {seq} already landed with hash {existing_hash} — occupied seqs \
                 are never superseded; the assembled whole must stay unambiguous"
            )))
        }
        Some(_) => {
            let landed = temper_substrate::uploads::landed_segments(pool, caller, upload_id)
                .await
                .map_err(|e| ApiError::internal_scrubbed("blob upload read failed", e))?
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
        .map_err(|e| ApiError::internal_scrubbed("blob upload read failed", e))?
        .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;
    Ok(progress_from(upload_id, landed))
}

/// Finalize a staged upload: standing re-run (authoritative: standing can change
/// mid-upload, and it gates the put), concurrency tokens checked (a mismatch is
/// [`ApiError::Conflict`] — resumable, staging kept), the staged whole re-checked against
/// the cap BEFORE anything is assembled or put (the operator may have lowered it
/// mid-upload — the wrapper's refusal must never cost a provider put), then assembly in
/// seq order, hash, and exactly the S2 commit path — optional integrity hash checked (a
/// mismatch is [`ApiError::ContentIntegrity`] — the ingest precedent's face for "the
/// assembled bytes do not hash to the declaration"), readability-gated dedup pre-check,
/// provider put unless deduped, then `commit_blob` whose cap/allowlist refusals surface
/// verbatim. Staging dies on success only; every failure keeps it (keep-and-declare — a
/// TTL reaper is a declared hole, never silently clean).
pub async fn finalize_upload(
    pool: &PgPool,
    store: &dyn temper_substrate::blob_store::BlobStore,
    config: &crate::config::BlobConfig,
    caller: ProfileId,
    upload_id: Uuid,
    req: &BlobUploadFinalizeRequest,
    surface: Surface,
) -> ApiResult<BlobUploadFinalizeOutcome> {
    let session = temper_substrate::uploads::load_session(pool, caller, upload_id)
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob upload read failed", e))?
        .ok_or_else(|| ApiError::NotFound("upload not found".to_string()))?;

    // Auth before writes, again: the begin-time standing was a fail-fast courtesy; this
    // is the gate the put answers to.
    check_home_standing(pool, caller, &session.home).await?;

    let landed = temper_substrate::uploads::landed_segments(pool, caller, upload_id)
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob upload read failed", e))?
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

    // The staging ceiling held per append, so the whole is at most the operator's number —
    // UNLESS the operator lowered the cap mid-upload and the already-staged total now
    // exceeds it. Refuse BEFORE anything is assembled or put: an over-cap whole can never
    // exist to reach the provider, and the provider put preceding the wrapper's refusal is
    // exactly the orphan-bytes leak the review's F4 named. The wrapper stays the
    // commit-time authority (D9); this is the ordering that keeps its refusal bytes-free.
    if total_bytes > config.max_bytes {
        return Err(ApiError::BadRequest(format!(
            "blob_upload: the staged whole is {total_bytes} bytes against a per-blob cap of \
             {} — the commit refuses it and nothing was uploaded; begin a new upload within \
             the cap",
            config.max_bytes
        )));
    }

    let body = temper_substrate::uploads::assemble_body(pool, upload_id)
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob upload assemble failed", e))?;
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

    // Home-scoped dedup pre-check (D2 as amended): the staging session's own home answers.
    let deduped =
        temper_substrate::readback::home_blob_id_by_hash(pool, &session.home, &content_hash)
            .await
            .map_err(|e| ApiError::internal_scrubbed("blob dedup pre-check failed", e))?
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
            .map_err(|e| ApiError::internal_scrubbed("blob provider put failed", e))?;
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, surface.marker())
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob emitter resolve failed", e))?;

    let blob_id = temper_substrate::writes::commit_blob(
        pool,
        store,
        temper_substrate::writes::CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: session.home,
            owner: caller,
            originator: None,
            content_hash: content_hash.clone(),
            content_type: session.content_type,
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
        .map_err(|e| ApiError::internal_scrubbed("blob upload cleanup failed", e))?;

    // N2: the stored media type, read back — on a dedup hit the row is the FIRST
    // committer's, and the response must say what is stored, not what was declared
    // (`stored_content_type`, the single site both committing doors share).
    let stored_type = stored_content_type(pool, blob_id.uuid())
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob commit read failed", e))?;

    Ok(BlobUploadFinalizeOutcome {
        blob_id,
        content_hash,
        deduped,
        content_type: stored_type,
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
    BlobRelationEdgeDirection as WireRelationDirection, BlobRelationRow as WireRelationRow,
    BlobSummary as WireBlobSummary,
};
use temper_core::types::graph::{EdgeKind as WireEdgeKind, Polarity as WirePolarity};

/// The blob's home anchor, readable-gated: `None` means the blob is absent OR not the
/// caller's — indistinguishable, the 404 either way (a probe over blob ids learns
/// nothing). This is the existence gate and the home read in ONE query, so the gate cannot
/// pass while the home read fails — the S3 `landed_segments` lesson applied at the shape
/// level: the gate is the row fetch, never a filter emptied afterwards. The home rides the
/// row (D2 as amended — one row, one home).
async fn blob_home(
    pool: &PgPool,
    caller: ProfileId,
    blob: BlobId,
) -> ApiResult<Option<(temper_substrate::payloads::AnchorTable, uuid::Uuid)>> {
    let row = sqlx::query!(
        r#"SELECT b.home_table, b.home_id
             FROM kb_blobs b
            WHERE b.id = $1
              AND blob_readable_by_profile($2, $1)"#,
        blob.uuid(),
        caller.uuid(),
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let table = match r.home_table.as_str() {
            "kb_contexts" => temper_substrate::payloads::AnchorTable::Contexts,
            _ => temper_substrate::payloads::AnchorTable::Cogmaps,
        };
        (table, r.home_id)
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
        temper_substrate::payloads::AnchorTable::Contexts => {
            sqlx::query_scalar!(
                "SELECT context_authorable_by_profile($1, $2)",
                caller.uuid(),
                anchor_id,
            )
            .fetch_one(pool)
            .await?
        }
        _ => {
            sqlx::query_scalar!(
                "SELECT cogmap_authorable_by_profile($1, $2)",
                caller.uuid(),
                anchor_id,
            )
            .fetch_one(pool)
            .await?
        }
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
    .await?;
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
    surface: Surface,
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
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, surface.marker())
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob emitter resolve failed", e))?;
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
    .map_err(|e| ApiError::internal_scrubbed("blob relation assert failed", e))?;

    Ok(WireRelationAck {
        edge_handle: edge.uuid(),
    })
}

/// List the blobs the caller can read, optionally scoped to one home anchor. The gate is
/// the substrate's set read — `blob_readable_by_profile`, the NAMED predicate — so this
/// surface honors visibility and cannot redefine it (the register's list-surfaces arm).
///
/// The optional home scope's parse lives here (the `parse_home` rule): the pair constraint
/// and the two-kind vocabulary are stated once, and every surface passes its wire strings
/// straight through.
pub async fn list_blobs(
    pool: &PgPool,
    caller: ProfileId,
    home_table: Option<String>,
    home_id: Option<Uuid>,
) -> ApiResult<Vec<WireBlobSummary>> {
    let home = parse_home_scope(home_table, home_id)?;
    let rows = temper_substrate::readback::blobs_readable_by_profile(pool, caller, home.as_ref())
        .await
        .map_err(|e| ApiError::internal_scrubbed("blob list failed", e))?;
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
        .map_err(|e| ApiError::internal_scrubbed("blob relations read failed", e))?
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
                direction: parse_wire_relation_direction(&r.direction)?,
                weight: r.weight,
                created: r.created,
            })
        })
        .collect()
}

/// The SQL enum's value set is fixed by DDL; a miss means the DB and the code disagree,
/// which is a 500, never a caller-facing refusal. Routed through `internal_scrubbed`
/// (the F9 choke point, like every other internal bail here): the raw DB value is
/// log-only — the wire carries the same generic internal message either way.
fn parse_wire_edge_kind(s: &str) -> ApiResult<WireEdgeKind> {
    match s {
        "express" => Ok(WireEdgeKind::Express),
        "contains" => Ok(WireEdgeKind::Contains),
        "leads_to" => Ok(WireEdgeKind::LeadsTo),
        "near" => Ok(WireEdgeKind::Near),
        other => Err(ApiError::internal_scrubbed(
            "unknown edge_kind value in kb_edges",
            other,
        )),
    }
}

fn parse_wire_polarity(s: &str) -> ApiResult<WirePolarity> {
    match s {
        "forward" => Ok(WirePolarity::Forward),
        "inverse" => Ok(WirePolarity::Inverse),
        other => Err(ApiError::internal_scrubbed(
            "unknown edge_polarity value in kb_edges",
            other,
        )),
    }
}

/// The relations listing's direction is not a DDL enum — it is the readback's own SQL
/// CASE literal (`outgoing`/`incoming`) — but the treatment is the same (C-C3,
/// 2026-09-04 review): the pair is typed on the wire ([`WireRelationDirection`], the
/// typed-structs rule), and a miss here is a readback/code disagreement, scrubbed at the
/// F9 choke point like its siblings. Shared with `edge_service` (the resource-side
/// listing parses the same CASE literal into the same wire enum) so the two listings
/// cannot drift on the parse.
pub(crate) fn parse_wire_relation_direction(s: &str) -> ApiResult<WireRelationDirection> {
    match s {
        "outgoing" => Ok(WireRelationDirection::Outgoing),
        "incoming" => Ok(WireRelationDirection::Incoming),
        other => Err(ApiError::internal_scrubbed(
            "unknown direction value in blob relation readback",
            other,
        )),
    }
}

/// The optional home scope for the list, in `parse_home`'s terms: a home is a
/// kb_contexts or kb_cogmaps anchor, and the refusal says so in one voice.
fn parse_home_scope(
    home_table: Option<String>,
    home_id: Option<Uuid>,
) -> ApiResult<Option<temper_substrate::payloads::AnchorRef>> {
    use temper_substrate::payloads::{AnchorRef, AnchorTable};
    match (home_table, home_id) {
        (None, None) => Ok(None),
        (Some(table), Some(id)) => {
            let anchor_table = match table.as_str() {
                "kb_contexts" => AnchorTable::Contexts,
                "kb_cogmaps" => AnchorTable::Cogmaps,
                other => {
                    return Err(ApiError::BadRequest(format!(
                        "blob_list: a home anchor is a kb_contexts or kb_cogmaps anchor — got \
                         home table {other}"
                    )))
                }
            };
            Ok(Some(AnchorRef {
                table: anchor_table,
                id,
            }))
        }
        (table, id) => Err(ApiError::BadRequest(format!(
            "blob_list: home_table and home_id are a pair — got home_table {} and home_id {}",
            table.as_deref().unwrap_or("<absent>"),
            id.map(|u| u.to_string())
                .unwrap_or_else(|| "<absent>".into())
        ))),
    }
}
