//! The v2 internal signature: its known-answer vector, and what it binds.
//!
//! These live here rather than in `src/internal_sig.rs`'s test module because their fixtures are
//! fixed nonces, and CodeQL's `rust/hard-coded-cryptographic-value` reads a literal nonce as a
//! hard-coded one wherever it sits. Rust has no inline suppression, and
//! `.github/codeql/codeql-config.yml` excludes `crates/*/tests/**` for exactly this reason: in a test,
//! every secret and nonce is a fixture the test wrote. A known-answer vector cannot be anything else.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use temper_core::internal_sig::{
    nonce_is_well_formed, sign_v2, verify_v2, DOOR_PRINCIPAL_RESOLVE, DOOR_SAML_RECONCILE,
};

const KAT_SECRET: &[u8] = b"topsecret-abcdefghijklmnopqrstuvwxyz012345";
const KAT_TIMESTAMP: i64 = 1_750_000_000;
const KAT_BODY: &[u8] = br#"{"provider":"saml:acme","external_user_id":"nid-1","email":"a@corp.io","email_verified":true,"idp_key":"acme","groups":["engineering"]}"#;

/// The retired `"{timestamp}.{body}"` construction's signature over the KAT inputs.
const KAT_V1_SIG: &str = "41eed1973f8f2e35fa65ff4e300f076fa08c206ca6c51434bae7d8a0c827d485";

/// The retired `"{timestamp}.{body}"` construction, built here because nothing in `src/` signs it
/// any more. It exists only so a test can show it is not a v2 signature.
fn sign_v1(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

// The v2 known-answer vector, asserted identically in
// packages/temper-cloud/tests/oauth/wire-contract.test.ts and
// packages/agent-workflows/mention/tests/link.test.ts.
const KAT_V2_NONCE: &str = "0123456789abcdef0123456789abcdef";
const KAT_V2_SIG: &str = "47385733b968d6043c77d0f515723b670636b69b40e3678e8fe82dfef890c65a";

#[test]
fn sign_v2_matches_known_answer_vector() {
    assert_eq!(
        sign_v2(
            KAT_SECRET,
            "POST",
            DOOR_SAML_RECONCILE,
            KAT_TIMESTAMP,
            KAT_V2_NONCE,
            KAT_BODY
        ),
        KAT_V2_SIG
    );
}

#[test]
fn v2_is_bound_to_its_door() {
    let sig = sign_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
    );
    assert!(verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
        &sig
    ));
    assert!(!verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_PRINCIPAL_RESOLVE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
        &sig
    ));
    assert!(!verify_v2(
        KAT_SECRET,
        "PUT",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
        &sig
    ));
}

#[test]
fn v2_is_bound_to_its_nonce_and_never_equals_v1() {
    let sig = sign_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
    );
    assert!(!verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        "fedcba9876543210fedcba9876543210",
        KAT_BODY,
        &sig
    ));
    assert_ne!(sig, KAT_V1_SIG);
    assert_eq!(sign_v1(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY), KAT_V1_SIG);
    assert!(!verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
        KAT_V1_SIG
    ));
}

#[test]
fn v2_is_bound_to_its_body_key_and_timestamp() {
    let sig = sign_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
    );
    let tampered = br#"{"provider":"saml:acme","external_user_id":"nid-1","email":"a@corp.io","email_verified":true,"idp_key":"acme","groups":["admins"]}"#;
    assert!(!verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        tampered,
        &sig
    ));
    assert!(!verify_v2(
        b"different-secret",
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
        &sig
    ));
    assert!(!verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP + 1,
        KAT_V2_NONCE,
        KAT_BODY,
        &sig
    ));
}

#[test]
fn v2_refuses_malformed_hex_without_panicking() {
    assert!(!verify_v2(
        KAT_SECRET,
        "POST",
        DOOR_SAML_RECONCILE,
        KAT_TIMESTAMP,
        KAT_V2_NONCE,
        KAT_BODY,
        "not-hex-zz"
    ));
}

/// The deprecated 0.6.x items still compute exactly what they always did, so a downstream caller
/// sees no change before their removal in 0.7.0. Nothing in temper calls them.
#[test]
#[allow(deprecated)]
fn the_deprecated_v1_items_are_unchanged_until_removal() {
    use temper_core::internal_sig::{sign, verify, SIGNATURE_HEADER};

    assert_eq!(SIGNATURE_HEADER, "X-Temper-Signature");
    assert_eq!(sign(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY), KAT_V1_SIG);
    assert!(verify(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY, KAT_V1_SIG));
    assert!(!verify(KAT_SECRET, KAT_TIMESTAMP + 1, KAT_BODY, KAT_V1_SIG));
    assert!(!verify(KAT_SECRET, KAT_TIMESTAMP, KAT_BODY, "not-hex-zz"));
}

#[test]
fn nonce_shape() {
    assert!(nonce_is_well_formed(KAT_V2_NONCE));
    assert!(!nonce_is_well_formed("0123456789ABCDEF0123456789ABCDEF"));
    assert!(!nonce_is_well_formed("0123456789abcdef"));
    assert!(!nonce_is_well_formed("0123456789abcdef0123456789abcdeg"));
}
