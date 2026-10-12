//! The internal signature gates' verdict: a v2 signature is good for one door, once.
//!
//! [`temper_core::internal_sig`] owns the construction; this module owns the decision, because
//! "once" needs the database. Both temper-api's report gates (reconcile, resolve, link-state) and
//! the mint's sealed constructor ([`super::slack_mint_service::verify_mint_request`]) call
//! [`accept_once`], so no gate can verify a signature without also spending its nonce.
//!
//! Only the v2 construction is read. The original `X-Temper-Signature` is ignored here even though
//! signers still send it: a verifier that accepted either would leave the original's gaps open to
//! anyone who strips the v2 headers from a capture.
//!
//! Checks, in order, each before anything costlier: a fresh timestamp, a well-formed nonce, the MAC
//! over this method and door, then the nonce claim. The claim comes last so that only a request
//! signed with the door's key can write a row. Every refusal answers the same way on the wire —
//! the caller learns only "invalid" or "stale", as before — and the log names which it was.

use sqlx::PgPool;

use temper_core::internal_sig::{self, DOORS, MAX_SKEW_SECS};

use crate::error::{ApiError, ApiResult};

/// One signed request as a gate received it.
pub struct SignedRequest<'a> {
    /// The HTTP method the request arrived with.
    pub method: &'a str,
    /// The route the request matched — the door the signature must name.
    pub door: &'a str,
    /// `X-Temper-Timestamp`.
    pub timestamp: i64,
    /// `X-Temper-Nonce`.
    pub nonce: &'a str,
    /// `X-Temper-Signature-V2`.
    pub signature: &'a str,
    /// The raw body bytes, exactly as received.
    pub body: &'a [u8],
}

/// Why a signed request was refused. Never sent to the caller; it is the log's distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The timestamp is outside [`MAX_SKEW_SECS`] of now.
    Stale,
    /// The nonce is not [`internal_sig::NONCE_LEN`] lowercase hex.
    MalformedNonce,
    /// The signature verifies for no door under this key.
    Mismatch,
    /// The signature verifies, but for a different door than the one it was sent to.
    CrossDoor(&'static str),
    /// The signature verifies and its nonce was already accepted.
    Replayed,
}

impl Refusal {
    /// The bounded reason recorded in the refusal's log line.
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::Stale => "stale",
            Refusal::MalformedNonce => "malformed_nonce",
            Refusal::Mismatch => "signature_mismatch",
            Refusal::CrossDoor(_) => "cross_door",
            Refusal::Replayed => "replayed",
        }
    }

    /// The wire answer: the same two messages the gates have always given.
    fn into_api_error(self) -> ApiError {
        match self {
            Refusal::Stale => ApiError::Unauthorized("stale internal signature".to_string()),
            _ => ApiError::Unauthorized("invalid internal signature".to_string()),
        }
    }
}

/// Everything short of the nonce claim: freshness, nonce shape, and the MAC over this door.
///
/// A signature that fails for this door is checked against the other doors, so a body signed for
/// one door and sent to another is told apart from a forgery in the log.
pub fn check(secret: &[u8], req: &SignedRequest<'_>, now: i64) -> Result<(), Refusal> {
    if !internal_sig::timestamp_is_fresh(req.timestamp, now) {
        return Err(Refusal::Stale);
    }
    if !internal_sig::nonce_is_well_formed(req.nonce) {
        return Err(Refusal::MalformedNonce);
    }
    let verifies_for = |door: &str| {
        internal_sig::verify_v2(
            secret,
            req.method,
            door,
            req.timestamp,
            req.nonce,
            req.body,
            req.signature,
        )
    };
    if verifies_for(req.door) {
        return Ok(());
    }
    match DOORS.iter().find(|d| **d != req.door && verifies_for(d)) {
        Some(door) => Err(Refusal::CrossDoor(door)),
        None => Err(Refusal::Mismatch),
    }
}

/// Record `nonce` as spent at `door`, unless it already is. Returns `false` for a nonce already
/// recorded there.
///
/// The row is kept until the signature can no longer be fresh: its timestamp plus twice the skew
/// window. Expiry and the prune are both judged on the API's clock (`now`), the clock freshness is
/// judged on, never the database's: a database clock running ahead would otherwise prune a row
/// while its signature was still fresh to the API, and the replay would be claimed again. Twice the
/// window covers API instances whose clocks disagree by up to `MAX_SKEW_SECS`. Expired rows are
/// deleted in the same statement, so the table holds about a minute of traffic.
async fn claim_nonce(
    pool: &PgPool,
    door: &str,
    nonce: &str,
    timestamp: i64,
    now: i64,
) -> ApiResult<bool> {
    let retain_secs = (timestamp + 2 * MAX_SKEW_SECS) as f64;
    let now_secs = now as f64;
    let claimed = sqlx::query_scalar!(
        r#"WITH pruned AS (
               DELETE FROM kb_internal_signature_nonces WHERE expires_at < to_timestamp($4)
           )
           INSERT INTO kb_internal_signature_nonces (nonce, door, expires_at)
           VALUES ($1, $2, to_timestamp($3))
           ON CONFLICT (door, nonce) DO NOTHING
           RETURNING nonce"#,
        nonce,
        door,
        retain_secs,
        now_secs,
    )
    .fetch_optional(pool)
    .await?;
    Ok(claimed.is_some())
}

/// Log a refusal at `label`'s gate and turn it into the wire answer.
fn refuse(req: &SignedRequest<'_>, label: &str, refusal: Refusal) -> ApiError {
    match refusal {
        Refusal::CrossDoor(signed_for) => tracing::warn!(
            door = req.door,
            signed_for,
            reason = refusal.reason(),
            "{label}: rejected (signed for another door)"
        ),
        _ => tracing::warn!(
            door = req.door,
            reason = refusal.reason(),
            "{label}: rejected"
        ),
    }
    refusal.into_api_error()
}

/// [`check`], logged at `label`'s gate. A refusal is [`ApiError::Unauthorized`] with the message
/// the gates have always used.
pub fn verify(secret: &[u8], req: &SignedRequest<'_>, now: i64, label: &str) -> ApiResult<()> {
    check(secret, req, now).map_err(|refusal| refuse(req, label, refusal))
}

/// Spend a verified request's nonce, refusing one already spent. Call only after [`verify`]: the
/// claim is what makes a row, and only a request signed with the door's key may make one.
pub async fn spend(pool: &PgPool, req: &SignedRequest<'_>, now: i64, label: &str) -> ApiResult<()> {
    if claim_nonce(pool, req.door, req.nonce, req.timestamp, now).await? {
        Ok(())
    } else {
        Err(refuse(req, label, Refusal::Replayed))
    }
}

/// Accept a v2-signed request at most once: [`verify`], then [`spend`].
pub async fn accept_once(
    pool: &PgPool,
    secret: &[u8],
    req: &SignedRequest<'_>,
    now: i64,
    label: &str,
) -> ApiResult<()> {
    verify(secret, req, now, label)?;
    spend(pool, req, now, label).await
}
