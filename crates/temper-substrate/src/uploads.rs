//! Staged blob uploads: the row mechanics of segmented upload (spec: binary blobs,
//! 2026-09-01, D7 — begin/append/finalize over the same content-addressed target, the
//! segmented-INGEST precedent in blob terms).
//!
//! **Pre-ledger, by design** (migration `20260903000040` is the DDL's own contract): the
//! staged bytes never ride events — `blob_commit` refuses any bytes argument outright, so
//! there is no projector, no payload type, and deliberately no replay story for these
//! tables. The ledger's business begins at finalize, when the assembled whole first has a
//! hash. `replay.rs` diffs its enumerated tables only; this pair is excluded, and that
//! exclusion is a decision, not an omission.
//!
//! **The gate is owner-equality, never `blob_readable_by_profile`.** A staged session is
//! not a blob (it has no hash yet), not a resource, not an edge: caller-private until
//! finalized, so every read here takes the owner in its WHERE and returns `None` for a
//! session that is absent *or not the caller's* — the two are indistinguishable by
//! design. `blob_readable_by_profile` is the read gate for COMMITTED blobs
//! (`20260903000030` names it for those surfaces); do not reach for it here.

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::ids::ProfileId;
use crate::payloads::{AnchorRef, AnchorTable};

/// A staged upload session: the begin-time declaration plus ownership. The home and
/// content type are re-examined at finalize (standing by the service — it can change
/// mid-upload; the allowlist by the SQL wrapper — the sole authority, D9).
#[derive(Debug, Clone)]
pub struct StagedUpload {
    pub id: Uuid,
    pub owner_profile_id: ProfileId,
    pub home: AnchorRef,
    pub content_type: String,
}

/// One landed segment, as the progress read reports it.
#[derive(Debug, Clone)]
pub struct LandedSegment {
    pub seq: i32,
    /// Bare sha256 hex of the segment's raw bytes — the idempotent-append identity.
    pub segment_hash: String,
    pub segment_bytes: i64,
}

/// What an append did. Same segment at an occupied seq is the idempotent no-op; a
/// DIFFERENT segment there is a conflict — the assembled whole must stay unambiguous,
/// so an occupied seq is never superseded. `OverCeiling` is the staging ceiling refusing:
/// `staged` is the total the append WOULD have produced, `ceiling` the bound in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendOutcome {
    Landed,
    AlreadyLanded { segment_hash: String },
    Conflict { existing_hash: String },
    OverCeiling { staged: i64, ceiling: i64 },
}

/// Begin a staged upload. The id is server-minted: a staging session is not a ledger
/// entity, so there is no identity-as-input question (contrast `kb_blobs.id`'s deliberate
/// lack of a DEFAULT).
pub async fn create_session(
    pool: &PgPool,
    owner: ProfileId,
    home: &AnchorRef,
    content_type: &str,
) -> Result<Uuid> {
    let row = sqlx::query!(
        r#"INSERT INTO kb_blob_uploads (owner_profile_id, home_table, home_id, content_type)
           VALUES ($1, $2, $3, $4)
           RETURNING id"#,
        owner.uuid(),
        home.table.as_str(),
        home.id,
        content_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(row.id)
}

/// The session row's `home_table` back into the anchor table. The DDL CHECK admits only
/// the two homed-in kinds (the D2 home gate), so anything else is a constraint violation
/// — a schema bug, not a runtime input — and naming the CHECK is the honest arm.
fn home_table_to_anchor(s: &str) -> AnchorTable {
    match s {
        "kb_contexts" => AnchorTable::Contexts,
        "kb_cogmaps" => AnchorTable::Cogmaps,
        other => unreachable!(
            "kb_blob_uploads.home_table CHECK admits only kb_contexts/kb_cogmaps, got {other}"
        ),
    }
}

/// Read one staged session, owner-gated: `None` when the session is absent OR belongs to
/// someone else — the caller renders both as the same not-found (a probe over upload ids
/// learns nothing either way).
pub async fn load_session(
    pool: &PgPool,
    caller: ProfileId,
    upload_id: Uuid,
) -> Result<Option<StagedUpload>> {
    let row = sqlx::query!(
        r#"SELECT id, owner_profile_id, home_table, home_id, content_type
             FROM kb_blob_uploads
            WHERE id = $1 AND owner_profile_id = $2"#,
        upload_id,
        caller.uuid(),
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| StagedUpload {
        id: r.id,
        owner_profile_id: ProfileId::from(r.owner_profile_id),
        home: AnchorRef {
            table: home_table_to_anchor(&r.home_table),
            id: r.home_id,
        },
        content_type: r.content_type,
    }))
}

/// The currently-landed segment set, oldest first — the resume/progress read and the
/// source of the finalize echo. Owner-gated like every other read here: `None` when the
/// session is absent or not the caller's — an EMPTY landed set is `Some(vec![])`, a
/// genuinely different answer (a begun-but-empty session), and conflating the two would
/// hand an outsider a 200 where absence is the contract.
pub async fn landed_segments(
    pool: &PgPool,
    caller: ProfileId,
    upload_id: Uuid,
) -> Result<Option<Vec<LandedSegment>>> {
    let owned = sqlx::query!(
        "SELECT true AS \"owned!\" FROM kb_blob_uploads WHERE id = $1 AND owner_profile_id = $2",
        upload_id,
        caller.uuid(),
    )
    .fetch_optional(pool)
    .await?;
    if owned.is_none() {
        return Ok(None);
    }
    let rows = sqlx::query!(
        r#"SELECT seq, segment_hash, octet_length(bytes)::bigint AS "segment_bytes!"
             FROM kb_blob_upload_segments
            WHERE upload_id = $1
            ORDER BY seq"#,
        upload_id,
    )
    .fetch_all(pool)
    .await?;
    Ok(Some(
        rows.into_iter()
            .map(|r| LandedSegment {
                seq: r.seq,
                segment_hash: r.segment_hash,
                segment_bytes: r.segment_bytes,
            })
            .collect(),
    ))
}

/// Append one segment under the staging ceiling, ATOMICALLY. The whole decision — owner
/// gate, staged-total read, ceiling check, insert — runs in one transaction under the
/// session row's `FOR UPDATE` lock, so N concurrent appends from the owning principal
/// serialize instead of each reading the same staged total and all landing (the TOCTOU
/// the security review's F4 found: the over-cap whole was assembled in RAM and put to
/// the provider before the commit-time cap refused). The ceiling is the staging bound —
/// how many bytes one session may hold between begin and finalize — passed in by the
/// service from the same operator config the commit-time cap reads.
///
/// Race-free by the primary key: the INSERT either lands (new seq), or `ON CONFLICT DO
/// NOTHING` falls through to a re-read that distinguishes the idempotent no-op (same
/// hash) from the conflict (different bytes at an occupied seq).
pub async fn append_segment(
    pool: &PgPool,
    caller: ProfileId,
    upload_id: Uuid,
    seq: i32,
    bytes: &[u8],
    segment_hash: &str,
    staging_ceiling: i64,
) -> Result<Option<AppendOutcome>> {
    let mut txn = pool.begin().await?;

    // Owner gate AND the serialization point in one statement: a session that is absent
    // or not the caller's is the same None, and a found row is locked against every
    // other append to this session until this transaction ends.
    let owned = sqlx::query!(
        r#"SELECT true AS "owned!"
             FROM kb_blob_uploads
            WHERE id = $1 AND owner_profile_id = $2
            FOR UPDATE"#,
        upload_id,
        caller.uuid(),
    )
    .fetch_optional(&mut *txn)
    .await?;
    if owned.is_none() {
        return Ok(None);
    }

    let staged: i64 = sqlx::query!(
        r#"SELECT COALESCE(SUM(octet_length(bytes)), 0)::bigint AS "sum!"
             FROM kb_blob_upload_segments
            WHERE upload_id = $1"#,
        upload_id,
    )
    .fetch_one(&mut *txn)
    .await?
    .sum;
    let would_total = staged + bytes.len() as i64;
    if would_total > staging_ceiling {
        return Ok(Some(AppendOutcome::OverCeiling {
            staged: would_total,
            ceiling: staging_ceiling,
        }));
    }

    let landed = sqlx::query!(
        r#"INSERT INTO kb_blob_upload_segments (upload_id, seq, bytes, segment_hash)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (upload_id, seq) DO NOTHING
           RETURNING seq"#,
        upload_id,
        seq,
        bytes,
        segment_hash,
    )
    .fetch_optional(&mut *txn)
    .await?;

    if landed.is_some() {
        sqlx::query!(
            "UPDATE kb_blob_uploads SET updated = now() WHERE id = $1",
            upload_id
        )
        .execute(&mut *txn)
        .await?;
        txn.commit().await?;
        return Ok(Some(AppendOutcome::Landed));
    }

    let existing = sqlx::query!(
        "SELECT segment_hash FROM kb_blob_upload_segments WHERE upload_id = $1 AND seq = $2",
        upload_id,
        seq,
    )
    .fetch_one(&mut *txn)
    .await?;
    txn.commit().await?;
    if existing.segment_hash == segment_hash {
        Ok(Some(AppendOutcome::AlreadyLanded {
            segment_hash: existing.segment_hash,
        }))
    } else {
        Ok(Some(AppendOutcome::Conflict {
            existing_hash: existing.segment_hash,
        }))
    }
}

/// Assemble the staged whole in seq order. Bounded by the staging ceiling `append_segment`
/// enforces atomically per append — the sessions' staged total can never exceed the
/// operator's number, so the materialization here is bounded by it, under concurrency as
/// anywhere else.
pub async fn assemble_body(pool: &PgPool, upload_id: Uuid) -> Result<Vec<u8>> {
    let rows = sqlx::query!(
        "SELECT bytes FROM kb_blob_upload_segments WHERE upload_id = $1 ORDER BY seq",
        upload_id,
    )
    .fetch_all(pool)
    .await?;
    let mut body = Vec::with_capacity(rows.iter().map(|r| r.bytes.len()).sum());
    for r in rows {
        body.extend_from_slice(&r.bytes);
    }
    Ok(body)
}

/// Delete the session and its segments (the segments row cascades). Called on finalize
/// success; every finalize failure leaves the staging in place (resumable — the
/// keep-and-declare posture; a TTL reaper is a declared hole, not silently clean).
pub async fn delete_session(pool: &PgPool, upload_id: Uuid) -> Result<()> {
    sqlx::query!("DELETE FROM kb_blob_uploads WHERE id = $1", upload_id)
        .execute(pool)
        .await?;
    Ok(())
}
