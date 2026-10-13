#![cfg(feature = "test-db")]
//! The Slack grant vault's seal format, pinned: a grant sealed by an earlier binary must open in this
//! one.
//!
//! `mint_access_token` answers a row that will not open under the key that sealed it with
//! `not_vaulted` (a re-link), on the premise that such a row is the ROW's fault. A change to how rows
//! are opened (the AAD layout, the field tags, the cipher) would fail every row that way and offer
//! every linked user a re-link, so it has to fail the build instead.
//!
//! The nonces and ciphertexts below are fixed bytes computed OUTSIDE this code: HChaCha20 in Python
//! (checked against the XChaCha draft's test vector), then `cryptography`'s ChaCha20-Poly1305, under
//! key `[3; 32]` with AAD `<principal>\0rt` and `<principal>\0at`. They live in a test tree rather
//! than beside the code because a fixed nonce in `src/` is, correctly, a CodeQL finding.
//!
//! Each test goes through the public read paths, so it pins the real open: `mint_access_token` opens
//! the cached access token, `take_refresh_token_locked` the refresh token.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sqlx::PgPool;
use uuid::Uuid;

use temper_services::services::grant_crypto::VaultKey;
use temper_services::services::slack_grant_vault_service::{
    mint_access_token, store_grant, take_refresh_token_locked, MintOutcome, NewGrant, RetiredKeys,
};

const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

const RT_NONCE: [u8; 24] = [0x11; 24];
const RT_SEALED: &str = "80Tufn1JNLvHIxkeNVa8kphjNkwKUtfjCD7f6QwFxFYT/GoH";
const AT_NONCE: [u8; 24] = [0x22; 24];
const AT_SEALED: &str = "Cl31qvX+oeLyla/RMFKvn4j+d4u30wdLBQozPZrvHZp2snM=";

fn key() -> VaultKey {
    VaultKey::from_base64(&STANDARD.encode([3u8; 32])).expect("key")
}

/// An approved, Slack-linked profile with a grant whose ciphertexts are replaced by the pinned ones.
/// The grant is stored through `store_grant` first so every other column is what production writes.
async fn pinned_grant(pool: &PgPool) {
    let profile = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
        .bind(profile)
        .bind(format!("user-{profile}"))
        .execute(pool)
        .await
        .expect("profile");
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'approved')")
        .bind(profile)
        .execute(pool)
        .await
        .expect("standing");
    sqlx::query(
        "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
         VALUES ($1, $2, 'slack', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(profile)
    .bind(PRINCIPAL)
    .execute(pool)
    .await
    .expect("link");

    let owner = temper_services::test_support::human_principal_for(pool, profile).await;
    let mut conn = pool.acquire().await.expect("acquire");
    store_grant(
        &mut conn,
        &key(),
        NewGrant {
            owner: &owner,
            slack_principal_id: PRINCIPAL,
            refresh_token: "replaced",
            access_token: "replaced",
            access_ttl_secs: Some(3600),
        },
    )
    .await
    .expect("store");

    sqlx::query(
        "UPDATE kb_slack_grant_vault \
            SET rt_nonce = $2, rt_ciphertext = $3, at_nonce = $4, at_ciphertext = $5 \
          WHERE slack_principal_id = $1",
    )
    .bind(PRINCIPAL)
    .bind(&RT_NONCE[..])
    .bind(STANDARD.decode(RT_SEALED).expect("rt vector"))
    .bind(&AT_NONCE[..])
    .bind(STANDARD.decode(AT_SEALED).expect("at vector"))
    .execute(pool)
    .await
    .expect("pin the ciphertexts");
}

/// **Why this bites:** a change to `aad` or the `at` field tag fails the open, the row carries the
/// configured key's fingerprint, and the mint answers `not_vaulted` instead of the pinned token.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cached_token_sealed_by_an_earlier_binary_still_mints(pool: PgPool) {
    pinned_grant(&pool).await;
    // Unreachable token URL: the cached branch must take, so no refresh is attempted.
    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
        "c",
        PRINCIPAL,
    )
    .await
    .expect("mint");
    match out {
        MintOutcome::Token { access_token, .. } => assert_eq!(access_token, "pinned-access-token"),
        other => panic!("the pinned access token must open, got {other:?}"),
    }
}

/// **Why this bites:** a change to `aad` or the `rt` field tag fails the open, and
/// `take_refresh_token_locked` reports an unopenable row as `None`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_refresh_token_sealed_by_an_earlier_binary_still_opens(pool: PgPool) {
    pinned_grant(&pool).await;
    let mut conn = pool.acquire().await.expect("acquire");
    let rt = take_refresh_token_locked(&mut conn, &key(), PRINCIPAL)
        .await
        .expect("take");
    assert_eq!(rt.as_deref(), Some("pinned-refresh-token"));
}
