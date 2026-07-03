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

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Header carrying the Unix-seconds timestamp the signature was computed over.
pub const TIMESTAMP_HEADER: &str = "X-Temper-Timestamp";

/// Header carrying the lowercase-hex HMAC-SHA256 signature.
pub const SIGNATURE_HEADER: &str = "X-Temper-Signature";

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
