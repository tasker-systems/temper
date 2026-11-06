//! A context's changes after a position: the service behind every door to the change feed.
//!
//! The read itself is `context_changes_after` (20261116000010, last replaced by 20261117000040):
//! a context's domain events under the settled horizon, in commit order, each a pointer (kind,
//! subject, actor, time, act) and never content. This service adds what a door needs on top of it:
//!
//! - **The gate first.** A context the reader cannot read is refused exactly as `GET
//!   /api/contexts/{id}` refuses it (404, `context_service::CONTEXT_REFUSAL`), before the position,
//!   bound or kinds are looked at, so none of them is disclosed for a context the reader cannot
//!   read. (A query string that does not parse at all, such as a non-numeric `limit`, is rejected
//!   by the HTTP framework before any handler runs, identically for every context.)
//! - **An opaque position.** The substrate's position is `(xact_id, seq)`, and both expose
//!   cluster-wide write volume. Every position a door hands out is sealed with XChaCha20-Poly1305
//!   under the deployment's one feed key (`kb_feed_position_keys`, 20261117000000), with the
//!   context as associated data. A position that is malformed, forged, sealed under another key or
//!   issued for another context fails the same tag and is refused as one that this deployment did
//!   not issue for this context. It is never read as the beginning.
//! - **A position ahead of the ledger is refused.** A position names a transaction at or below the
//!   horizon it was read under, and the horizon never moves back. One above the present horizon can
//!   only come from a database that has moved back (a restore), and reading from it would skip every
//!   change between the two. A restored database that has since passed that transaction again is
//!   NOT detected, and its readers skip the changes in between: that limit is stated on the goal.
//!
//! A position names a place in the context's ledger, not a reader: anyone who can read the
//! context may resume from any position issued for it. The first read on a deployment mints the
//! key (one row, once); every other read writes nothing.
//!
//! **The consumer contract.** The feed is a change hint, not a cache-invalidation guarantee. It
//! never announces an admin act, so a consumer that mirrors content must periodically re-read the
//! subjects it holds and drop what its read now refuses. Every door repeats this.
//!
//! **What the feed withholds besides admin acts** (20261117000020, 20261117000040).
//! - An edge is served only while the reader can read both its endpoints, as `edges_visible_to`
//!   answers, so an edge whose endpoint leaves the reader's view goes quiet rather than reporting a
//!   fold. When that endpoint lived in this context, its own departure is served here. When it
//!   lives in another context, nothing here says so, and the periodic re-read drops the edge.
//! - A connection's traffic, `webhook_received` and `subscription_delivery_disposed`, is never
//!   served, and naming either in `kinds` is refused as an unknown kind (`_feed_withholds_kind`).
//! - A resource leaves a context one way: a deletion and a move are each one `resource_departed`,
//!   naming only the resource, so an exit never says whether the resource still exists elsewhere.
//!   `resource_deleted` stays in the ledger, is not served, and is refused in `kinds` the same way.
//!
//! **What an entry names.** A change to a content block names the block's resource as its subject
//! and the block as `block`: a block is re-read through its resource. Every subject is one of
//! `kb_resources`, `kb_edges`, `kb_blobs` or `kb_contexts`; an event naming anything else is
//! withheld, never served on a guess.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use rand::Rng as _;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service::CONTEXT_REFUSAL;
use temper_core::types::api::{ContextChange, ContextChangesResponse};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::query_params::ContextChangesQuery;

/// The page bound when none is given (F6).
pub const DEFAULT_LIMIT: i32 = 100;
/// The largest page bound a reader may ask for (F6). A larger one is refused, never clamped.
pub const MAX_LIMIT: i32 = 1000;

/// The most kinds one read may name. Well above the ledger's domain kinds; it bounds what a reader
/// can make the filter compare against on every scanned event.
const MAX_KINDS: usize = 64;

/// Every position this deployment issues starts with this. The version is in the prefix so a later
/// encoding can never be mistaken for this one.
const POSITION_PREFIX: &str = "fp1_";
/// Authenticated with the context id as the associated data. Versioned with the prefix.
const POSITION_AAD_DOMAIN: &[u8] = b"temper-feed-position/v1";
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
/// `xact_id` and `seq`, eight bytes each, big-endian.
const PLAINTEXT_LEN: usize = 16;

/// The refusal for any position that does not open under this deployment's key for this context:
/// malformed, forged, from another deployment or another context. AEAD cannot tell those apart, and
/// the reader needs nothing more than to start again.
pub const POSITION_REFUSAL: &str =
    "position is not one this deployment issued for this context; omit `after` to start again";
/// The refusal for a position whose transaction is above this deployment's present horizon.
pub const POSITION_AHEAD_REFUSAL: &str =
    "position is ahead of this deployment's ledger; omit `after` to start again";

/// A position inside the substrate's order: `(xact_id, seq)`. Never leaves this module unsealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawPosition {
    xact: u64,
    seq: u64,
}

fn aad(context: ContextId) -> Vec<u8> {
    let mut aad = POSITION_AAD_DOMAIN.to_vec();
    aad.extend_from_slice(context.uuid().as_bytes());
    aad
}

fn seal(key: &[u8; KEY_LEN], context: ContextId, pos: RawPosition) -> ApiResult<String> {
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let mut nonce = [0u8; NONCE_LEN];
    UnwrapErr(SysRng).fill_bytes(&mut nonce);
    let mut plain = [0u8; PLAINTEXT_LEN];
    plain[..8].copy_from_slice(&pos.xact.to_be_bytes());
    plain[8..].copy_from_slice(&pos.seq.to_be_bytes());
    let aad = aad(context);
    let sealed = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &plain,
                aad: &aad,
            },
        )
        .map_err(|_| ApiError::Internal("feed position seal failed".to_string()))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(format!("{POSITION_PREFIX}{}", URL_SAFE_NO_PAD.encode(out)))
}

/// `None` for anything that is not a position this key sealed for this context.
fn open(key: &[u8; KEY_LEN], context: ContextId, token: &str) -> Option<RawPosition> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token.strip_prefix(POSITION_PREFIX)?)
        .ok()?;
    if bytes.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, sealed) = bytes.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let aad = aad(context);
    let plain = cipher
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: sealed,
                aad: &aad,
            },
        )
        .ok()?;
    let plain: [u8; PLAINTEXT_LEN] = plain.as_slice().try_into().ok()?;
    Some(RawPosition {
        xact: u64::from_be_bytes(plain[..8].try_into().ok()?),
        seq: u64::from_be_bytes(plain[8..].try_into().ok()?),
    })
}

fn key_from_row(bytes: Vec<u8>) -> ApiResult<[u8; KEY_LEN]> {
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| ApiError::Internal("feed position key is not 32 bytes".to_string()))
}

/// The deployment's feed key, if one has been minted. Opening never mints: with no key, no
/// position was ever issued, so any position given is refused.
async fn existing_key(pool: &PgPool) -> ApiResult<Option<[u8; KEY_LEN]>> {
    let row = sqlx::query_scalar!(r#"SELECT key FROM kb_feed_position_keys WHERE id = 1"#)
        .fetch_optional(pool)
        .await?;
    row.map(key_from_row).transpose()
}

/// The deployment's feed key, minting it on first use. Two first readers race on the insert and
/// both read back whichever row won, so they seal under the same key.
async fn key_minting(pool: &PgPool) -> ApiResult<[u8; KEY_LEN]> {
    if let Some(key) = existing_key(pool).await? {
        return Ok(key);
    }
    let mut fresh = [0u8; KEY_LEN];
    UnwrapErr(SysRng).fill_bytes(&mut fresh);
    sqlx::query!(
        r#"INSERT INTO kb_feed_position_keys (id, key) VALUES (1, $1) ON CONFLICT (id) DO NOTHING"#,
        fresh.as_slice()
    )
    .execute(pool)
    .await?;
    existing_key(pool)
        .await?
        .ok_or_else(|| ApiError::Internal("feed position key was not stored".to_string()))
}

/// The kinds filter, parsed and checked against the ledger's domain event types. An unknown name
/// is refused: a typo must not read as "nothing of that kind changed".
async fn parse_kinds(pool: &PgPool, raw: Option<&str>) -> ApiResult<Option<Vec<String>>> {
    let Some(raw) = raw else { return Ok(None) };
    let mut kinds: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    if kinds.len() > MAX_KINDS {
        return Err(ApiError::BadRequest(format!(
            "`kinds` names more than {MAX_KINDS} kinds; omit it for every kind"
        )));
    }
    if kinds.is_empty() {
        return Err(ApiError::BadRequest(
            "`kinds` names no kind; omit it for every kind".to_string(),
        ));
    }
    let known: Vec<String> = sqlx::query_scalar!(
        r#"SELECT name AS "name!" FROM kb_event_types
            WHERE category = 'domain' AND name = ANY($1) AND NOT _feed_withholds_kind(name)"#,
        &kinds
    )
    .fetch_all(pool)
    .await?;
    if let Some(unknown) = kinds.iter().find(|k| !known.contains(k)) {
        return Err(ApiError::BadRequest(format!(
            "unknown change kind `{unknown}`"
        )));
    }
    Ok(Some(kinds))
}

#[derive(serde::Deserialize)]
struct SubstratePosition {
    xact: String,
    seq: i64,
}

#[derive(serde::Deserialize)]
struct SubstrateAnswer {
    entries: Vec<ContextChange>,
    has_more: bool,
    through: SubstratePosition,
}

/// Read `context`'s changes after `query.after`, as `profile`. See the module note for the
/// refusals and the consumer contract.
pub async fn changes_after(
    pool: &PgPool,
    profile: ProfileId,
    context: ContextId,
    query: &ContextChangesQuery,
) -> ApiResult<ContextChangesResponse> {
    let readable = sqlx::query_scalar!(
        r#"SELECT context_readable_by_profile($1, $2) AS "readable!""#,
        *profile,
        *context,
    )
    .fetch_one(pool)
    .await?;
    if !readable {
        return Err(ApiError::NotFound(CONTEXT_REFUSAL.to_string()));
    }

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "`limit` must be between 1 and {MAX_LIMIT}"
        )));
    }
    let kinds = parse_kinds(pool, query.kinds.as_deref()).await?;

    let after = match query.after.as_deref() {
        None => None,
        Some(token) => {
            let key = existing_key(pool)
                .await?
                .ok_or_else(|| ApiError::BadRequest(POSITION_REFUSAL.to_string()))?;
            let pos = open(&key, context, token)
                .ok_or_else(|| ApiError::BadRequest(POSITION_REFUSAL.to_string()))?;
            let behind_horizon = sqlx::query_scalar!(
                r#"SELECT $1::text::xid8 <= pg_snapshot_xmin(pg_current_snapshot()) AS "ok!""#,
                pos.xact.to_string(),
            )
            .fetch_one(pool)
            .await?;
            if !behind_horizon {
                return Err(ApiError::BadRequest(POSITION_AHEAD_REFUSAL.to_string()));
            }
            Some(pos)
        }
    };

    let seq = after
        .map(|p| i64::try_from(p.seq))
        .transpose()
        .map_err(|_| ApiError::BadRequest(POSITION_REFUSAL.to_string()))?;
    let answer = sqlx::query_scalar!(
        r#"SELECT context_changes_after($1, $2, $3::text::xid8, $4, $5, $6) AS "answer""#,
        *profile,
        *context,
        after.map(|p| p.xact.to_string()),
        seq,
        limit,
        kinds.as_deref(),
    )
    .fetch_one(pool)
    .await?;
    // NULL is the substrate's own gate refusing: standing was lost between the two reads.
    let Some(answer) = answer else {
        return Err(ApiError::NotFound(CONTEXT_REFUSAL.to_string()));
    };
    let answer: SubstrateAnswer = serde_json::from_value(answer)
        .map_err(|e| ApiError::Internal(format!("change feed answer shape: {e}")))?;

    let through = RawPosition {
        xact: answer
            .through
            .xact
            .parse()
            .map_err(|e| ApiError::Internal(format!("change feed position: {e}")))?,
        seq: u64::try_from(answer.through.seq)
            .map_err(|e| ApiError::Internal(format!("change feed position: {e}")))?,
    };
    let key = key_minting(pool).await?;
    Ok(ContextChangesResponse {
        entries: answer.entries,
        through: seal(&key, context, through)?,
        has_more: answer.has_more,
    })
}

/// Seal an arbitrary position under this deployment's key, so a witness can hand the door a
/// position no read issued (one ahead of the ledger). Absent from production builds.
#[cfg(feature = "test-db")]
pub async fn position_for_test(pool: &PgPool, context: ContextId, xact: u64, seq: u64) -> String {
    let key = key_minting(pool).await.expect("feed key");
    seal(&key, context, RawPosition { xact, seq }).expect("seal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];

    fn ctx() -> ContextId {
        ContextId::from(Uuid::now_v7())
    }

    #[test]
    fn a_position_opens_only_for_the_context_and_key_it_was_sealed_for() {
        let (a, b) = (ctx(), ctx());
        let pos = RawPosition {
            xact: 9_000_017,
            seq: 42,
        };
        let token = seal(&KEY, a, pos).unwrap();
        assert_eq!(open(&KEY, a, &token), Some(pos));
        assert_eq!(open(&KEY, b, &token), None, "another context's position");
        assert_eq!(
            open(&[8; KEY_LEN], a, &token),
            None,
            "another key's position"
        );
    }

    #[test]
    fn a_position_shows_neither_number() {
        let pos = RawPosition {
            xact: 9_000_017,
            seq: 42,
        };
        let token = seal(&KEY, ctx(), pos).unwrap();
        assert!(
            !token.contains("9000017") && !token.contains("42"),
            "{token}"
        );
        assert_ne!(
            seal(&KEY, ctx(), pos).unwrap(),
            token,
            "fresh nonce per seal"
        );
    }

    #[test]
    fn a_damaged_or_foreign_position_never_opens() {
        let a = ctx();
        let token = seal(&KEY, a, RawPosition { xact: 1, seq: 0 }).unwrap();
        let mut flipped = token.clone().into_bytes();
        let last = flipped.len() - 1;
        flipped[last] = if flipped[last] == b'A' { b'B' } else { b'A' };
        for bad in [
            String::new(),
            "fp1_".to_string(),
            token.trim_start_matches(POSITION_PREFIX).to_string(),
            format!("fp2_{}", token.trim_start_matches(POSITION_PREFIX)),
            String::from_utf8(flipped).unwrap(),
            "fp1_!!!!".to_string(),
        ] {
            assert_eq!(open(&KEY, a, &bad), None, "{bad:?}");
        }
    }
}
