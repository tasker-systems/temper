//! Request signing for temper's internal server-to-server doors.
//!
//! The signers are the SAML Authorization Server (`packages/temper-cloud/src/oauth/reconcile.ts`)
//! and the Slack mention agent (`packages/agent-workflows/mention/agent/lib/link.ts`). Each signs
//! with an HMAC over the **raw request body** plus a timestamp, so the shared secret never crosses
//! the wire.
//!
//! **We MAC the raw body bytes as transmitted — not a re-serialized form.** The
//! signer HMACs the exact JSON bytes it sends; the verifier HMACs the exact
//! bytes it received (buffered before deserialization). Because both operate on
//! identical bytes there is no cross-language canonicalization to drift on — the
//! same discipline every major webhook signature uses (GitHub `X-Hub-Signature-256`,
//! Stripe). The TS signers and this module are pinned together by a shared known-answer vector
//! (`tests/internal_sig_v2.rs`, `tests/oauth/wire-contract.test.ts`, `mention/tests/link.test.ts`).
//!
//! # The construction: one door, once
//!
//! ```text
//! "v2\n{METHOD}\n{door}\n{timestamp}\n{nonce}\n{body}"
//! ```
//!
//! The message names the method, the door and a per-request nonce, so a signature is good for one
//! door, and a verifier that records the nonce accepts it once. The signature is lowercase-hex
//! HMAC-SHA256.
//!
//! `door` is the route as temper-api registers it (the `DOOR_*` constants), never the URL the
//! signer posts to: a deployment may reach the API through a prefix or rewrite, and the route is
//! the one name both sides hold. `nonce` is [`NONCE_LEN`] lowercase hex characters of fresh
//! randomness. Headers: [`TIMESTAMP_HEADER`] + [`NONCE_HEADER`] + [`SIGNATURE_V2_HEADER`].
//!
//! The `v2` prefix and header name are kept from the cutover off an earlier `"{timestamp}.{body}"`
//! construction sent as `X-Temper-Signature`. No signer sends that construction and no verifier
//! reads it; the tests that prove it is refused build it themselves. Its public items remain below,
//! deprecated, only so this crate's 0.6.x line keeps its API; they are removed in 0.7.0.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Header carrying the Unix-seconds timestamp the signature was computed over.
pub const TIMESTAMP_HEADER: &str = "X-Temper-Timestamp";

/// Header carrying the v2 construction's per-request nonce.
pub const NONCE_HEADER: &str = "X-Temper-Nonce";

/// Header carrying the v2 construction's lowercase-hex HMAC-SHA256 signature.
pub const SIGNATURE_V2_HEADER: &str = "X-Temper-Signature-V2";

/// Length of a v2 nonce: 128 bits of randomness as lowercase hex.
pub const NONCE_LEN: usize = 32;

/// The SAML Authorization Server's membership reconcile.
pub const DOOR_SAML_RECONCILE: &str = "/internal/saml/reconcile";
/// The SAML Authorization Server's principal resolve. Same key as [`DOOR_SAML_RECONCILE`].
pub const DOOR_PRINCIPAL_RESOLVE: &str = "/internal/principal/resolve";
/// The mention agent's link-state question.
pub const DOOR_SLACK_LINK_STATE: &str = "/internal/slack/link-state";
/// The mention agent's act-as-the-human mint.
pub const DOOR_SLACK_MINT: &str = "/internal/slack/mint";

/// Every door the internal signature gates. A verifier that refuses a v2 signature checks it
/// against the others, so a body sent to the wrong door is reported as that, not as a forgery.
pub const DOORS: [&str; 4] = [
    DOOR_SAML_RECONCILE,
    DOOR_PRINCIPAL_RESOLVE,
    DOOR_SLACK_LINK_STATE,
    DOOR_SLACK_MINT,
];

/// Maximum accepted clock skew (seconds) between signer and verifier. A request
/// whose timestamp is further than this from "now" (in either direction) is
/// rejected as stale, which is what makes a captured request non-replayable.
pub const MAX_SKEW_SECS: i64 = 30;

/// Feed the v2 message into a fresh MAC. Newline separates the fields: none of method, door,
/// timestamp or nonce can contain one, and the body comes last, so no field can absorb another.
fn mac_message_v2(
    secret: &[u8],
    method: &str,
    door: &str,
    timestamp: i64,
    nonce: &str,
    body: &[u8],
) -> HmacSha256 {
    // HMAC accepts a key of any length, so this never errors.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"v2\n");
    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(door.as_bytes());
    mac.update(b"\n");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(nonce.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac
}

/// Compute the lowercase-hex v2 signature: `body` sent by `method` to `door`, at `timestamp`,
/// under `nonce`.
pub fn sign_v2(
    secret: &[u8],
    method: &str,
    door: &str,
    timestamp: i64,
    nonce: &str,
    body: &[u8],
) -> String {
    hex::encode(
        mac_message_v2(secret, method, door, timestamp, nonce, body)
            .finalize()
            .into_bytes(),
    )
}

/// Constant-time-verify a presented v2 signature. Returns `false` on any mismatch, including
/// malformed hex — never panics. Single use is the caller's: this checks the MAC only.
pub fn verify_v2(
    secret: &[u8],
    method: &str,
    door: &str,
    timestamp: i64,
    nonce: &str,
    body: &[u8],
    presented_hex: &str,
) -> bool {
    let Ok(presented) = hex::decode(presented_hex) else {
        return false;
    };
    // `verify_slice` is a constant-time comparison of the computed tag.
    mac_message_v2(secret, method, door, timestamp, nonce, body)
        .verify_slice(&presented)
        .is_ok()
}

/// The three v2 headers for `body` sent by `method` to `door` at `timestamp` under `nonce`, as
/// `(name, value)` pairs. The signers outside Rust build the same three.
pub fn v2_headers(
    secret: &[u8],
    method: &str,
    door: &str,
    timestamp: i64,
    nonce: &str,
    body: &[u8],
) -> [(&'static str, String); 3] {
    [
        (TIMESTAMP_HEADER, timestamp.to_string()),
        (NONCE_HEADER, nonce.to_owned()),
        (
            SIGNATURE_V2_HEADER,
            sign_v2(secret, method, door, timestamp, nonce, body),
        ),
    ]
}

/// Whether `nonce` has the v2 shape: exactly [`NONCE_LEN`] lowercase hex characters.
pub fn nonce_is_well_formed(nonce: &str) -> bool {
    nonce.len() == NONCE_LEN
        && nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether a signed `timestamp` is within [`MAX_SKEW_SECS`] of `now` (Unix
/// seconds), tolerating skew in either direction.
///
/// `abs_diff` rather than `(now - timestamp).abs()`: the subtraction wraps in a release build, and
/// a timestamp near `i64::MIN` away from now would wrap to a "difference" that passes.
pub fn timestamp_is_fresh(timestamp: i64, now: i64) -> bool {
    now.abs_diff(timestamp) <= MAX_SKEW_SECS as u64
}

// The retired `"{timestamp}.{body}"` construction. Kept for the 0.6.x line only, so a patch release
// removes no public item; nothing in temper calls these.

/// Header that carried the retired `"{timestamp}.{body}"` signature.
#[deprecated(
    since = "0.6.1",
    note = "no temper verifier reads X-Temper-Signature; sign with `v2_headers`. Removed in 0.7.0."
)]
pub const SIGNATURE_HEADER: &str = "X-Temper-Signature";

/// Feed the retired `"{timestamp}.{body}"` message into a fresh MAC.
fn mac_message_v1(secret: &[u8], timestamp: i64, body: &[u8]) -> HmacSha256 {
    // HMAC accepts a key of any length, so this never errors.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac
}

/// The retired `"{timestamp}.{body}"` signature, lowercase hex. No temper verifier accepts it.
#[deprecated(
    since = "0.6.1",
    note = "no temper verifier accepts this construction; use `sign_v2`. Removed in 0.7.0."
)]
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    hex::encode(
        mac_message_v1(secret, timestamp, body)
            .finalize()
            .into_bytes(),
    )
}

/// Constant-time check of a retired `"{timestamp}.{body}"` signature. Returns `false` on any
/// mismatch, including malformed hex. No temper verifier calls it.
#[deprecated(
    since = "0.6.1",
    note = "no temper verifier accepts this construction; use `verify_v2`. Removed in 0.7.0."
)]
pub fn verify(secret: &[u8], timestamp: i64, body: &[u8], presented_hex: &str) -> bool {
    let Ok(presented) = hex::decode(presented_hex) else {
        return false;
    };
    mac_message_v1(secret, timestamp, body)
        .verify_slice(&presented)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The construction's known-answer vector and what it binds are in `tests/internal_sig_v2.rs`:
    // their fixtures are fixed nonces, which CodeQL flags anywhere under `src/`.

    #[test]
    fn freshness_window_is_symmetric() {
        let now = 1_750_000_000;
        assert!(timestamp_is_fresh(now, now));
        assert!(timestamp_is_fresh(now - MAX_SKEW_SECS, now));
        assert!(timestamp_is_fresh(now + MAX_SKEW_SECS, now));
        assert!(!timestamp_is_fresh(now - MAX_SKEW_SECS - 1, now));
        assert!(!timestamp_is_fresh(now + MAX_SKEW_SECS + 1, now));
    }

    #[test]
    fn freshness_does_not_wrap_at_the_extremes() {
        let now: i64 = 1_750_000_000;
        assert!(!timestamp_is_fresh(now.wrapping_add(i64::MIN), now));
        assert!(!timestamp_is_fresh(i64::MIN, now));
        assert!(!timestamp_is_fresh(i64::MAX, now));
    }
}
