//! `intake_service::classify_hub_signature`: what a GitHub receipt's `X-Hub-Signature-256` says.
//!
//! Outside `src/` because the fixtures are fixed HMAC keys, which CodeQL's
//! `rust/hard-coded-cryptographic-value` reads as hard-coded wherever they sit; Rust has no inline
//! suppression, and `.github/codeql/codeql-config.yml` excludes `crates/*/tests/**` for this case.
//! None of these reach a database.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use temper_services::services::intake_service::{classify_hub_signature, HubSignature};

const SECRET: &str = "a-github-webhook-secret";
const BODY: &[u8] = br#"{"action":"opened","number":1}"#;

/// GitHub's header value for `body` under `secret`.
fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn classify(secret: Option<&str>, header: Option<&str>, body: &[u8]) -> HubSignature {
    classify_hub_signature(secret, header.map(str::as_bytes), body)
}

#[test]
fn no_header_is_absent_whether_or_not_a_secret_is_configured() {
    assert_eq!(classify(None, None, BODY), HubSignature::Absent);
    assert_eq!(classify(Some(SECRET), None, BODY), HubSignature::Absent);
}

#[test]
fn a_header_with_no_secret_configured_is_present() {
    assert_eq!(
        classify(None, Some(&sign(SECRET, BODY)), BODY),
        HubSignature::Present
    );
}

#[test]
fn the_signature_over_the_exact_bytes_is_valid() {
    assert_eq!(
        classify(Some(SECRET), Some(&sign(SECRET, BODY)), BODY),
        HubSignature::Valid
    );
}

/// An uppercase hex tag is the same tag.
#[test]
fn an_uppercase_hex_tag_is_valid() {
    let signed = sign(SECRET, BODY);
    let upper = format!("sha256={}", signed["sha256=".len()..].to_ascii_uppercase());
    assert_eq!(
        classify(Some(SECRET), Some(&upper), BODY),
        HubSignature::Valid
    );
}

#[test]
fn a_different_key_is_invalid() {
    assert_eq!(
        classify(Some(SECRET), Some(&sign("another-secret", BODY)), BODY),
        HubSignature::Invalid
    );
}

/// The question the probe exists to answer: the same JSON re-serialized is different bytes, and
/// does not verify.
#[test]
fn the_same_json_in_different_bytes_is_invalid() {
    let reserialized = br#"{"action": "opened", "number": 1}"#;
    assert_eq!(
        classify(Some(SECRET), Some(&sign(SECRET, BODY)), reserialized),
        HubSignature::Invalid
    );
}

#[test]
fn a_malformed_header_is_invalid() {
    let signed = sign(SECRET, BODY);
    let tag = &signed["sha256=".len()..];
    for value in [
        tag.to_string(),                     // no prefix
        format!("sha1={tag}"),               // the wrong algorithm
        format!("sha256={}", &tag[..62]),    // truncated
        format!("sha256={tag}00"),           // too long
        "sha256=not-hex-at-all".to_string(), // not hex
        "sha256=".to_string(),               // empty tag
    ] {
        assert_eq!(
            classify(Some(SECRET), Some(&value), BODY),
            HubSignature::Invalid,
            "{value:?} must not verify"
        );
    }
}

#[test]
fn a_non_utf8_header_is_invalid() {
    assert_eq!(
        classify_hub_signature(Some(SECRET), Some(b"sha256=\xff\xfe"), BODY),
        HubSignature::Invalid
    );
}
