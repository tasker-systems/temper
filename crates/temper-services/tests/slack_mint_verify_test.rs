//! The mint door's sealed constructor refuses before it spends a nonce.
//!
//! Outside `src/` because the fixtures are fixed nonces, which CodeQL's
//! `rust/hard-coded-cryptographic-value` reads as hard-coded wherever they sit; Rust has no inline
//! suppression, and `.github/codeql/codeql-config.yml` excludes `crates/*/tests/**` for this case.
//! None of these reach a database.

use sqlx::PgPool;
use temper_core::internal_sig::{self, DOOR_SLACK_MINT};
use temper_services::error::ApiError;
use temper_services::services::internal_sig_service::SignedRequest;
use temper_services::services::slack_mint_service::verify_mint_request;

const MINT_SECRET: &str = "mint-secret";
const LINK_SECRET: &str = "link-secret-DIFFERENT";
const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";
const NONCE: &str = "0123456789abcdef0123456789abcdef";
const NOW: i64 = 1_700_000_000;

/// A pool that never connects. Every refusal below happens before the nonce is spent, so none
/// of these tests reach the database; the accepted path and the replay are witnessed against a
/// real one in `tests/e2e/tests/slack_link_test.rs` (`mint_returns_a_token_for_a_vaulted_principal`,
/// `mint_refuses_a_replayed_request`).
fn unreachable_pool() -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unreachable.invalid/none")
        .expect("lazy pool")
}

/// A mint body and its v2 signature, as the agent's `signedHeaders` would produce.
fn signed(secret: &str, timestamp: i64, body: &[u8]) -> String {
    internal_sig::sign_v2(
        secret.as_bytes(),
        "POST",
        DOOR_SLACK_MINT,
        timestamp,
        NONCE,
        body,
    )
}

fn body_for(principal: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "slack_principal_id": principal }))
        .expect("serialize body")
}

fn request<'a>(timestamp: i64, body: &'a [u8], signature: &'a str) -> SignedRequest<'a> {
    SignedRequest {
        method: "POST",
        door: DOOR_SLACK_MINT,
        timestamp,
        nonce: NONCE,
        signature,
        body,
    }
}

/// Cross-gate containment: a request signed with a DIFFERENT secret (the link-state key) does
/// not verify, so a `SLACK_LINK_SECRET` holder cannot obtain a mint proof. This is the negative
/// test the design requires (spec §5.2) — the seal's only constructor gates on the mint secret.
#[tokio::test]
async fn a_link_secret_signature_cannot_mint_a_proof() {
    let body = body_for(PRINCIPAL);
    let sig = signed(LINK_SECRET, NOW, &body);
    let err = verify_mint_request(
        &unreachable_pool(),
        MINT_SECRET,
        &request(NOW, &body, &sig),
        NOW,
    )
    .await
    .expect_err("a link-secret signature must not mint a proof");
    assert!(
        matches!(err, ApiError::Unauthorized(_)),
        "wrong secret must be Unauthorized, got {err:?}",
    );
}

#[tokio::test]
async fn a_stale_timestamp_is_refused() {
    let signed_at = NOW;
    let now = signed_at + internal_sig::MAX_SKEW_SECS + 1;
    let body = body_for(PRINCIPAL);
    let sig = signed(MINT_SECRET, signed_at, &body);
    let err = verify_mint_request(
        &unreachable_pool(),
        MINT_SECRET,
        &request(signed_at, &body, &sig),
        now,
    )
    .await
    .expect_err("a stale timestamp must be refused");
    assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
}

/// The original construction is no longer a mint credential, whatever its freshness.
#[tokio::test]
async fn the_original_construction_is_refused() {
    let body = body_for(PRINCIPAL);
    let sig = internal_sig::sign(MINT_SECRET.as_bytes(), NOW, &body);
    let err = verify_mint_request(
        &unreachable_pool(),
        MINT_SECRET,
        &request(NOW, &body, &sig),
        NOW,
    )
    .await
    .expect_err("the original construction must be refused");
    assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
}

/// A well-signed request whose principal is malformed is a 400, NOT a 401 — the shape error must
/// stay distinct from the auth error (`mint_rejects_a_malformed_principal` asserts the 400).
#[tokio::test]
async fn a_malformed_principal_is_bad_request_not_unauthorized() {
    let body = body_for("not-a-slack-principal");
    let sig = signed(MINT_SECRET, NOW, &body);
    let err = verify_mint_request(
        &unreachable_pool(),
        MINT_SECRET,
        &request(NOW, &body, &sig),
        NOW,
    )
    .await
    .expect_err("a malformed principal must be refused");
    assert!(
        matches!(err, ApiError::BadRequest(_)),
        "a malformed principal must be BadRequest, got {err:?}",
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_is_bad_request() {
    let body = b"not json at all";
    let sig = signed(MINT_SECRET, NOW, body);
    let err = verify_mint_request(
        &unreachable_pool(),
        MINT_SECRET,
        &request(NOW, body, &sig),
        NOW,
    )
    .await
    .expect_err("a non-JSON body must be refused");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}
