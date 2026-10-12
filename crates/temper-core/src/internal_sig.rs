//! Request signing for the internal SAML reconcile channel (AS → temper-api).
//!
//! Replaces the earlier static shared-secret header with an HMAC signature over
//! the **raw request body** plus a timestamp. Two wins over sending the secret
//! itself: the secret never crosses the wire, and a captured request is
//! replay-proof (the verifier rejects a stale timestamp).
//!
//! **We MAC the raw body bytes as transmitted — not a re-serialized form.** The
//! signer HMACs the exact JSON bytes it sends; the verifier HMACs the exact
//! bytes it received (buffered before deserialization). Because both operate on
//! identical bytes there is no cross-language canonicalization to drift on — the
//! same discipline every major webhook signature uses (GitHub `X-Hub-Signature-256`,
//! Stripe). The TS signer (`packages/temper-cloud/src/oauth/reconcile.ts`) and this
//! module are pinned together by a shared known-answer vector (see the test below
//! and `tests/oauth/wire-contract.test.ts`).
//!
//! Message construction: `"{timestamp}.{body}"`, where `timestamp` is Unix
//! seconds and `body` is the raw request-body bytes. Signature is lowercase-hex
//! HMAC-SHA256. Headers: [`TIMESTAMP_HEADER`] + [`SIGNATURE_HEADER`].
//!
//! # The v2 construction: one door, once
//!
//! The construction above binds neither the door nor the occasion. A body signed for
//! `/internal/saml/reconcile` verifies at `/internal/principal/resolve`, which shares its key, and
//! a captured request verifies again for as long as its timestamp is fresh. [`sign_v2`] closes
//! both: its message names the method, the door and a per-request nonce, so a signature is good
//! for one door, and a verifier that records the nonce accepts it once.
//!
//! ```text
//! "v2\n{METHOD}\n{door}\n{timestamp}\n{nonce}\n{body}"
//! ```
//!
//! `door` is the route as temper-api registers it (the `DOOR_*` constants), never the URL the
//! signer posts to: a deployment may reach the API through a prefix or rewrite, and the route is
//! the one name both sides hold. `nonce` is [`NONCE_LEN`] lowercase hex characters of fresh
//! randomness. Headers: [`TIMESTAMP_HEADER`] (shared) + [`NONCE_HEADER`] + [`SIGNATURE_V2_HEADER`].
//!
//! The cutover runs on the signers, not the verifier. Signers send both constructions until every
//! verifier requires v2; a verifier that accepted either would leave the old construction's gaps
//! open to anyone who strips the v2 headers from a capture.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Header carrying the Unix-seconds timestamp the signature was computed over.
pub const TIMESTAMP_HEADER: &str = "X-Temper-Timestamp";

/// Header carrying the lowercase-hex HMAC-SHA256 signature.
pub const SIGNATURE_HEADER: &str = "X-Temper-Signature";

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

/// Feed the canonical `"{timestamp}.{body}"` message into a fresh MAC.
fn mac_message(secret: &[u8], timestamp: i64, body: &[u8]) -> HmacSha256 {
    // HMAC accepts a key of any length, so this never errors.
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac
}

/// Compute the lowercase-hex HMAC-SHA256 signature for `body` at `timestamp`.
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    hex::encode(mac_message(secret, timestamp, body).finalize().into_bytes())
}

/// Constant-time-verify a presented lowercase-hex signature against the body.
/// Returns `false` on any mismatch, including malformed hex — never panics.
pub fn verify(secret: &[u8], timestamp: i64, body: &[u8], presented_hex: &str) -> bool {
    let Ok(presented) = hex::decode(presented_hex) else {
        return false;
    };
    // `verify_slice` is a constant-time comparison of the computed tag.
    mac_message(secret, timestamp, body)
        .verify_slice(&presented)
        .is_ok()
}

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
    mac_message_v2(secret, method, door, timestamp, nonce, body)
        .verify_slice(&presented)
        .is_ok()
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
pub fn timestamp_is_fresh(timestamp: i64, now: i64) -> bool {
    (now - timestamp).abs() <= MAX_SKEW_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared known-answer vector. The identical inputs and expected signature are
    // asserted on the TS side in packages/temper-cloud/tests/oauth/wire-contract.test.ts,
    // so the two runtimes cannot drift on the HMAC construction.
    const KAT_SECRET: &[u8] = b"topsecret-abcdefghijklmnopqrstuvwxyz012345";
    const KAT_TIMESTAMP: i64 = 1_750_000_000;
    const KAT_BODY: &[u8] = br#"{"provider":"saml:acme","external_user_id":"nid-1","email":"a@corp.io","email_verified":true,"idp_key":"acme","groups":["engineering"]}"#;
    const KAT_SIG: &str = "41eed1973f8f2e35fa65ff4e300f076fa08c206ca6c51434bae7d8a0c827d485";

    #[test]
    fn sign_matches_known_answer_vector() {
        assert_eq!(sign(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY), KAT_SIG);
    }

    #[test]
    fn verify_accepts_matching_signature() {
        assert!(verify(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY, KAT_SIG));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let tampered = br#"{"provider":"saml:acme","external_user_id":"nid-1","email":"a@corp.io","email_verified":true,"idp_key":"acme","groups":["admins"]}"#;
        assert!(!verify(KAT_SECRET, KAT_TIMESTAMP, tampered, KAT_SIG));
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        assert!(!verify(
            b"different-secret",
            KAT_TIMESTAMP,
            KAT_BODY,
            KAT_SIG
        ));
    }

    #[test]
    fn verify_rejects_wrong_timestamp() {
        assert!(!verify(KAT_SECRET, KAT_TIMESTAMP + 1, KAT_BODY, KAT_SIG));
    }

    #[test]
    fn verify_rejects_malformed_hex() {
        assert!(!verify(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY, "not-hex-zz"));
    }

    #[test]
    fn freshness_window_is_symmetric() {
        let now = 1_750_000_000;
        assert!(timestamp_is_fresh(now, now));
        assert!(timestamp_is_fresh(now - MAX_SKEW_SECS, now));
        assert!(timestamp_is_fresh(now + MAX_SKEW_SECS, now));
        assert!(!timestamp_is_fresh(now - MAX_SKEW_SECS - 1, now));
        assert!(!timestamp_is_fresh(now + MAX_SKEW_SECS + 1, now));
    }
}
