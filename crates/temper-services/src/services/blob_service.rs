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
use temper_core::types::ids::{BlobId, ProfileId};
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
