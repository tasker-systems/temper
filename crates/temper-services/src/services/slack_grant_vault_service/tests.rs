use super::super::grant_crypto::VaultKey;
use super::*;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use temper_principal::Refusal;

const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

fn key() -> VaultKey {
    VaultKey::from_base64(&STANDARD.encode([3u8; 32])).unwrap()
}

/// Minimal profile insert — this suite tests the vault, not provisioning. The handle is the
/// FULL id: two UUIDv7s minted in the same millisecond share their first bytes, so a truncated
/// handle collides on `kb_profiles_handle_key` when a test inserts two profiles.
async fn insert_profile(pool: &PgPool) -> Uuid {
    let id = Uuid::now_v7();
    crate::test_support::profile::insert_with_id(pool, id, &format!("user-{id}")).await;
    // Mint is gated on `approved` standing (D-A), so a mintable fixture is an approved one.
    set_standing(pool, id, "approved").await;
    id
}

/// Upsert a principal's standing directly — the test-side analogue of the state machine, used
/// to drive the mint gate across `approved`/`revoked`/`deactivated` without an admin actor.
async fn set_standing(pool: &PgPool, profile: Uuid, state: &str) {
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1,$2) \
         ON CONFLICT (profile_id) DO UPDATE SET state = EXCLUDED.state",
    )
    .bind(profile)
    .bind(state)
    .execute(pool)
    .await
    .unwrap();
}

/// Seal a grant under the test key AND write the identity row that production always writes
/// alongside it. Wraps the [`NewGrant`] construction the tests would otherwise repeat.
///
/// The link row is load-bearing since the re-root (spec §2.3): `mint_access_token` now roots at
/// `kb_profile_auth_links`, so a vault row with no link reads as `NotLinked`. A real grant is
/// never linkless — the callback writes both in one transaction — so a faithful fixture links
/// and vaults the same profile+principal (which also keeps them correlated, never skewed).
///
/// `store_grant` takes `&mut PgConnection` (so the link callback can run it in the same
/// transaction as the directory write); these tests need no such atomicity, so they hand it a
/// one-off pooled connection.
async fn store(
    pool: &PgPool,
    profile: Uuid,
    principal: &str,
    rt: &str,
    at: &str,
    ttl: Option<u64>,
) {
    store_sealed_by(pool, &key(), profile, principal, rt, at, ttl).await;
}

/// [`store`], sealed under a given key: a deployment that held a different key at link time.
async fn store_sealed_by(
    pool: &PgPool,
    sealing_key: &VaultKey,
    profile: Uuid,
    principal: &str,
    rt: &str,
    at: &str,
    ttl: Option<u64>,
) {
    link(pool, profile, principal).await;
    let owner = crate::test_support::human_principal_for(pool, profile).await;
    let mut conn = pool.acquire().await.expect("acquire");
    store_grant(
        &mut conn,
        sealing_key,
        NewGrant {
            owner: &owner,
            slack_principal_id: principal,
            refresh_token: rt,
            access_token: at,
            access_ttl_secs: ttl,
        },
    )
    .await
    .unwrap();
}

/// Write the `kb_profile_auth_links` directory row binding a Slack principal to a profile —
/// the identity fact the re-rooted mint query starts from. Idempotent, so a test may link
/// without vaulting (to exercise the `NotVaulted` arm) and `store` may call it unconditionally.
async fn link(pool: &PgPool, profile: Uuid, principal: &str) {
    sqlx::query(
        "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) \
         VALUES ($1, $2, 'slack', $3) \
         ON CONFLICT (auth_provider, auth_provider_user_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(profile)
    .bind(principal)
    .execute(pool)
    .await
    .unwrap();
}

/// The stored refresh token is ENCRYPTED, not plaintext: the raw bytea column must not equal
/// the token. This is the whole "encrypted at rest" claim, asserted against the real column.
#[sqlx::test(migrations = "../../migrations")]
async fn store_seals_the_refresh_token_at_rest(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(
        &pool,
        profile,
        PRINCIPAL,
        "the-refresh-token",
        "the-at",
        Some(3600),
    )
    .await;

    let stored: Vec<u8> = sqlx::query_scalar!(
        "SELECT rt_ciphertext FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        PRINCIPAL,
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_ne!(
        stored, b"the-refresh-token",
        "the refresh token must be stored encrypted, never as plaintext bytes",
    );
}

/// A mint while the cached access token is still valid returns that token WITHOUT a refresh —
/// no network. This exercises the FOR UPDATE + decrypt-cached + return path end to end.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_returns_the_cached_token_without_refreshing(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(
        &pool,
        profile,
        PRINCIPAL,
        "rt",
        "cached-access-token",
        Some(3600),
    )
    .await;

    // token_url is deliberately unreachable: a passing test proves no refresh was attempted.
    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "client",
        PRINCIPAL,
    )
    .await
    .unwrap();
    // Assert the token AND that the expiry rides along pointing at the future. The expiry is
    // what the caller hands eve as `TokenResult.expiresAt`; a zero or past value would still
    // be "a token" and would still pass a token-only assertion, while making the runtime
    // refresh on every single call.
    match out {
        MintOutcome::Token {
            access_token,
            expires_at,
        } => {
            assert_eq!(access_token, "cached-access-token");
            assert!(
                expires_at > Utc::now(),
                "the cached mint must report a future expiry, got {expires_at}",
            );
        }
        other => panic!("expected the cached token, got {other:?}"),
    }
}

/// **The `expires_in` cast must not wrap.** A hostile or broken IdP answering with a huge
/// `expires_in` used to be cast `as i64` unchecked, wrapping NEGATIVE — so `access_expires_at`
/// landed in the PAST, the cached-token branch never took, and every mint spent a refresh,
/// maximizing exposure to the documented RT-rotation dual-write window.
///
/// **Why this bites:** `u64::MAX as i64` is `-1`, so without the clamp the stored expiry is one
/// second ago and the cached branch is skipped — mint then tries to refresh against a
/// deliberately unreachable `token_url` and the call ERRORS. Only the clamped version can hand
/// back the cached token. The stored expiry is asserted directly too, because a future change
/// that made mint tolerate a refresh failure would otherwise mask the wrap.
#[sqlx::test(migrations = "../../migrations")]
async fn a_huge_expires_in_is_clamped_rather_than_wrapping_negative(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(
        &pool,
        profile,
        PRINCIPAL,
        "rt",
        "cached-access-token",
        Some(u64::MAX),
    )
    .await;

    let stored: DateTime<Utc> = sqlx::query_scalar!(
        "SELECT access_expires_at FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        PRINCIPAL,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .expect("access_expires_at is written by store_grant");
    assert!(
        stored > Utc::now(),
        "a huge expires_in must clamp to a FUTURE expiry, not wrap to the past: {stored}",
    );

    // Unreachable token_url: a passing test proves the cached branch took, i.e. no refresh.
    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "client",
        PRINCIPAL,
    )
    .await
    .expect("a clamped expiry must serve the cached token without refreshing");
    match out {
        MintOutcome::Token { access_token, .. } => {
            assert_eq!(access_token, "cached-access-token");
        }
        other => panic!("expected the cached token, got {other:?}"),
    }
}

/// **FINDING C, the service half.** `store_grant` writes on the CALLER's connection, so it
/// shares that transaction's fate.
///
/// **Why this bites:** it rolls the transaction back and asserts the row is GONE. Against the
/// old `&PgPool` signature the INSERT was its own autocommit and survived the rollback — which
/// is exactly the property that let the callback commit a directory row and then lose the
/// grant to a process death.
#[sqlx::test(migrations = "../../migrations")]
async fn store_grant_is_rolled_back_with_its_caller_transaction(pool: PgPool) {
    let profile = insert_profile(&pool).await;

    let owner = crate::test_support::human_principal_for(&pool, profile).await;
    let mut tx = pool.begin().await.expect("begin");
    store_grant(
        &mut tx,
        &key(),
        NewGrant {
            owner: &owner,
            slack_principal_id: PRINCIPAL,
            refresh_token: "rt",
            access_token: "at",
            access_ttl_secs: Some(3600),
        },
    )
    .await
    .expect("store");
    tx.rollback().await.expect("rollback");

    let rows: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        PRINCIPAL,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or_default();
    assert_eq!(
        rows, 0,
        "the grant must share the caller's transaction — surviving a rollback is the \
         autocommit shape that made the callback's dual write unrecoverable",
    );
}

/// An unknown principal has no identity row, so the re-rooted query returns nothing → the mint
/// is `NotLinked`, distinct from `NotVaulted`. Before the re-root this arm could not exist: the
/// vault-rooted query conflated "never linked" with "linked, not vaulted".
#[sqlx::test(migrations = "../../migrations")]
async fn mint_reports_not_linked_for_an_unknown_principal(pool: PgPool) {
    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotLinked));
}

/// A link with no vaulted grant (a pre-T3 row) is `NotVaulted` — genuinely distinct from
/// `NotLinked`. This is the cell the whole re-root exists to separate.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_reports_not_vaulted_for_a_link_without_a_grant(pool: PgPool) {
    let profile = insert_profile(&pool).await; // approved
    link(&pool, profile, PRINCIPAL).await; // linked, but no `store` → no vault row

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
}

/// The ordering fix: a linked human whose standing refuses is told the STANDING reason, even
/// when there is also no vaulted grant. Before the re-root, "no vault" won and the human was
/// told to re-link — the false remedy, since only an admin approval helps.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_prefers_the_standing_refusal_over_a_missing_vault(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    set_standing(&pool, profile, "denied").await;
    link(&pool, profile, PRINCIPAL).await; // linked, denied, NOT vaulted

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        MintOutcome::Refused(LinkRefusal::Standing {
            refusal: Refusal::Denied
        }),
        "standing must win over vault state so the remedy named is the one that works",
    );
}

/// Fail-closed correlation (spec §2.3): when the link row and the vault row are owned by
/// DIFFERENT profiles (an account-merge skew), the join finds no vault for the link's owner, so
/// the mint reads as `NotVaulted` rather than decrypting one profile's grant under another's
/// standing. There is no production path that produces this today; the seed is manual.
#[sqlx::test(migrations = "../../migrations")]
async fn a_vault_owned_by_a_different_profile_than_the_link_reads_as_not_vaulted(pool: PgPool) {
    let link_owner = insert_profile(&pool).await; // approved
    let grant_owner = insert_profile(&pool).await; // approved
    link(&pool, link_owner, PRINCIPAL).await;
    // Vault the SAME principal, but owned by the OTHER profile — the skew.
    store(&pool, grant_owner, PRINCIPAL, "rt", "at", Some(3600)).await;

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        MintOutcome::Refused(LinkRefusal::NotVaulted),
        "a link/vault owner skew must fail closed, never mint across profiles",
    );
}

/// The former `revoked_at` disjunct is dropped (spec §2.4): a flagged `revoked_at` no longer
/// blocks minting, because soft-revoke was superseded by disconnect's DELETE (commit
/// `3a45b1ab`). The column is retained but inert; this pins that so a regression re-adding the
/// disjunct fails. A comfortably-valid cached AT keeps this off the (irrelevant) refresh path.
#[sqlx::test(migrations = "../../migrations")]
async fn a_flagged_revoked_at_no_longer_blocks_minting(pool: PgPool) {
    let profile = insert_profile(&pool).await; // approved
    store(
        &pool,
        profile,
        PRINCIPAL,
        "rt",
        "still-valid-at",
        Some(3600),
    )
    .await;
    sqlx::query!(
        "UPDATE kb_slack_grant_vault SET revoked_at = now() WHERE slack_principal_id = $1",
        PRINCIPAL,
    )
    .execute(&pool)
    .await
    .unwrap();

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    match out {
        MintOutcome::Token { access_token, .. } => assert_eq!(access_token, "still-valid-at"),
        other => panic!("a flagged revoked_at must no longer block minting, got {other:?}"),
    }
}

/// The deactivation kill-switch reaches the vault: a grant whose principal standing is
/// `deactivated` mints nothing, even with a still-valid cached token and without ever being
/// explicitly revoked.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_refuses_a_deactivated_profile(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    // A comfortably-valid cached AT — absent the standing gate, mint would hand it straight back.
    store(
        &pool,
        profile,
        PRINCIPAL,
        "rt",
        "still-valid-at",
        Some(3600),
    )
    .await;
    set_standing(&pool, profile, "deactivated").await;

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        MintOutcome::Refused(LinkRefusal::Standing {
            refusal: Refusal::Deactivated
        }),
        "a deactivated profile must not mint — the kill-switch must reach the vault",
    );
}

/// D-A: a `Revoke` stops minting immediately. This is the Phase-2 policy call — the mint gate is
/// `standing = 'approved'`, not the behaviour-preserving `<> 'deactivated'` — so a revoked
/// principal (whose cached token is otherwise still valid) mints nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_refuses_a_revoked_profile(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(
        &pool,
        profile,
        PRINCIPAL,
        "rt",
        "still-valid-at",
        Some(3600),
    )
    .await;
    set_standing(&pool, profile, "revoked").await;

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        MintOutcome::Refused(LinkRefusal::Standing {
            refusal: Refusal::Revoked
        }),
        "a revoked principal must not mint (D-A: mint requires an approved standing)",
    );
}

/// D-A / §8: a Slack-linked human born `Denied` (or with no standing row at all) cannot mint —
/// being born denied *is* the access control. Here the profile is left without an approved
/// standing, and even a fresh cached token is withheld.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_refuses_a_profile_without_approved_standing(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(
        &pool,
        profile,
        PRINCIPAL,
        "rt",
        "still-valid-at",
        Some(3600),
    )
    .await;
    set_standing(&pool, profile, "denied").await;

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        MintOutcome::Refused(LinkRefusal::Standing {
            refusal: Refusal::Denied
        }),
        "a born-Denied principal must not mint until approved (§8)",
    );
}

/// A ciphertext sealed for one principal cannot be opened under another, even by the same key:
/// the AAD binding defeats a row-transplant. We seal principal A's grant, copy A's rt
/// ciphertext+nonce into B's row (the exact move a DB-write attacker would make), expire B's
/// cached AT so mint must decrypt that transplanted RT, and require mint to FAIL CLOSED rather
/// than hand back A's refresh token under B.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_rejects_a_transplanted_ciphertext(pool: PgPool) {
    const PRINCIPAL_B: &str = "slack:T0BHAHEN79C:UATTACKER";
    let profile_a = insert_profile(&pool).await;
    let profile_b = insert_profile(&pool).await;
    store(
        &pool,
        profile_a,
        PRINCIPAL,
        "A-secret-rt",
        "A-at",
        Some(3600),
    )
    .await;
    // B's own grant, with an already-expired cached AT so mint is forced onto the RT path.
    store(&pool, profile_b, PRINCIPAL_B, "B-rt", "B-at", Some(0)).await;

    // Transplant A's sealed RT (ciphertext + nonce) into B's row.
    sqlx::query!(
        r#"
            UPDATE kb_slack_grant_vault dst
               SET rt_nonce = src.rt_nonce, rt_ciphertext = src.rt_ciphertext
              FROM kb_slack_grant_vault src
             WHERE dst.slack_principal_id = $1 AND src.slack_principal_id = $2
            "#,
        PRINCIPAL_B,
        PRINCIPAL,
    )
    .execute(&pool)
    .await
    .unwrap();

    // B's row still carries the configured key's fingerprint, so the failure is a row-level
    // fault: B is told to re-link (and an error is logged). What must never happen is a token.
    let result = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "c",
        PRINCIPAL_B,
    )
    .await;
    assert!(
        matches!(result, Ok(MintOutcome::Refused(LinkRefusal::NotVaulted))),
        "a ciphertext sealed for principal A must not decrypt under principal B — the AAD \
         binding must fail the tag rather than yield A's refresh token to B; got {result:?}",
    );
}

/// Re-storing the same principal upserts to ONE row and clears a prior revocation — a fresh
/// consent is a fresh, working grant.
#[sqlx::test(migrations = "../../migrations")]
async fn re_store_upserts_one_row_and_clears_revocation(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt1", "at1", Some(3600)).await;
    // Flip the flag directly — the write-side `revoke` fn is gone (disconnect now deletes
    // the row instead), but a re-store must still clear a row flagged before that change.
    sqlx::query!(
        "UPDATE kb_slack_grant_vault SET revoked_at = now() WHERE slack_principal_id = $1",
        PRINCIPAL,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Re-link: new grant, longer-lived cached AT.
    store(&pool, profile, PRINCIPAL, "rt2", "at2-fresh", Some(3600)).await;

    let rows: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        PRINCIPAL,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or_default();
    assert_eq!(rows, 1, "re-store must not duplicate the row");

    // The revocation is cleared and the new cached AT is served.
    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1/unused",
        "c",
        PRINCIPAL,
    )
    .await
    .unwrap();
    match out {
        MintOutcome::Token {
            access_token,
            expires_at,
        } => {
            assert_eq!(access_token, "at2-fresh");
            assert!(
                expires_at > Utc::now(),
                "a re-stored grant must serve a future expiry, got {expires_at}",
            );
        }
        other => panic!("expected the re-stored token, got {other:?}"),
    }
}

/// The principal is the key, WHOLE. A different principal has its own (here absent) identity
/// row, so it reads as `NotLinked` — it never reaches, let alone reads, PRINCIPAL's grant.
#[sqlx::test(migrations = "../../migrations")]
async fn mint_does_not_match_a_different_principal(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;

    let out = mint_access_token(
        &pool,
        &key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
        "c",
        "slack:T0BHAHEN79C:UOTHER",
    )
    .await
    .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotLinked));
}

// --- key fingerprints -------------------------------------------------------------------

/// A key this deployment does not hold: the one an operator mis-pasted, or the one rotated to.
fn other_key() -> VaultKey {
    VaultKey::from_base64(&STANDARD.encode([4u8; 32])).unwrap()
}

/// Every byte of a vault row the mint could write, for "nothing changed" assertions.
type RowBytes = (
    Vec<u8>,
    Vec<u8>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
);

async fn row_bytes(pool: &PgPool, principal: &str) -> RowBytes {
    let r = sqlx::query!(
        "SELECT rt_nonce, rt_ciphertext, at_nonce, at_ciphertext, key_fingerprint, \
                access_expires_at, updated_at \
           FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        principal,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (
        r.rt_nonce,
        r.rt_ciphertext,
        r.at_nonce,
        r.at_ciphertext,
        r.key_fingerprint,
        r.access_expires_at,
        r.updated_at,
    )
}

async fn stored_fingerprint(pool: &PgPool, principal: &str) -> Option<Vec<u8>> {
    sqlx::query_scalar!(
        "SELECT key_fingerprint FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        principal,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Make the row legacy: written before fingerprints existed.
async fn unstamp(pool: &PgPool, principal: &str) {
    sqlx::query!(
        "UPDATE kb_slack_grant_vault SET key_fingerprint = NULL WHERE slack_principal_id = $1",
        principal,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Flip one byte of the cached access-token ciphertext: tamper under the right key.
async fn tamper_at(pool: &PgPool, principal: &str) {
    sqlx::query!(
        "UPDATE kb_slack_grant_vault \
            SET at_ciphertext = set_byte(at_ciphertext, 0, get_byte(at_ciphertext, 0) # 255) \
          WHERE slack_principal_id = $1",
        principal,
    )
    .execute(pool)
    .await
    .unwrap();
}

fn retiring(raw: &str) -> RetiredKeys {
    RetiredKeys::parse(raw, key().fingerprint()).expect("valid retirement list")
}

async fn mint_with(
    pool: &PgPool,
    key: &VaultKey,
    retired: &RetiredKeys,
    token_url: &str,
) -> ApiResult<MintOutcome> {
    mint_access_token(pool, key, retired, token_url, "c", PRINCIPAL).await
}

#[sqlx::test(migrations = "../../migrations")]
async fn store_grant_stamps_the_sealing_keys_fingerprint(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
    assert_eq!(
        stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
        Some(key().fingerprint().as_bytes()),
    );
}

/// **The hazard PR #1120 reverted.** A wrong-but-well-formed key is a 500 that writes nothing,
/// so restoring the right key heals the grant.
///
/// **Why this bites:** mapping [`Unopenable::UnknownKey`] to `NotVaulted` (the reverted
/// behaviour) fails the first assertion. COMMITTING a write on the failure path (a stamp, a
/// reseal) fails the byte comparison; an uncommitted one rolls back with the transaction, which
/// is correct and does not fail it. And if the failure path overwrote the ciphertext, the right
/// key could no longer mint.
#[sqlx::test(migrations = "../../migrations")]
async fn a_wrong_key_is_a_500_that_leaves_the_row_for_the_right_key(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "cached-at", Some(3600)).await;
    let before = row_bytes(&pool, PRINCIPAL).await;

    let wrong = mint_with(
        &pool,
        &other_key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
    )
    .await;
    assert!(
        matches!(wrong, Err(ApiError::Internal(_))),
        "a key that did not seal the row and is not retired must be a 500, got {wrong:?}",
    );
    assert_eq!(
        row_bytes(&pool, PRINCIPAL).await,
        before,
        "a wrong-key mint must not write the row",
    );

    let healed = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
    match healed {
        Ok(MintOutcome::Token { access_token, .. }) => assert_eq!(access_token, "cached-at"),
        other => panic!("restoring the key must heal the grant, got {other:?}"),
    }
}

/// The same on the refresh path (no usable cached token): the refresh token is not spent, and
/// the row is not written.
///
/// **Why this bites:** the token URL is unreachable, so any attempt to refresh would also be an
/// `Err` — but an `Internal` one only by accident. The byte comparison is what bites: a stamp or
/// reseal before the open is classified changes the row.
#[sqlx::test(migrations = "../../migrations")]
async fn a_wrong_key_on_the_refresh_path_is_a_500_that_leaves_the_row(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
    let before = row_bytes(&pool, PRINCIPAL).await;

    let out = mint_with(
        &pool,
        &other_key(),
        &RetiredKeys::default(),
        "http://127.0.0.1:1",
    )
    .await;
    assert!(matches!(out, Err(ApiError::Internal(_))), "got {out:?}");
    assert_eq!(row_bytes(&pool, PRINCIPAL).await, before);
}

/// A deliberate rotation, acknowledged by naming the old key's fingerprint, turns a row sealed
/// under that key into `NotVaulted`, so the user gets a re-link URL.
///
/// **Why this bites:** without the retirement the same row is a 500 (asserted first), so only
/// consulting the retired set can produce the refusal.
#[sqlx::test(migrations = "../../migrations")]
async fn a_declared_rotation_mints_not_vaulted(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    // Sealed under the OLD key; the deployment now holds key().
    store_sealed_by(
        &pool,
        &other_key(),
        profile,
        PRINCIPAL,
        "rt",
        "at",
        Some(3600),
    )
    .await;

    let unacknowledged =
        mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
    assert!(
        matches!(unacknowledged, Err(ApiError::Internal(_))),
        "an unacknowledged rotation must stay a 500, got {unacknowledged:?}",
    );

    let retired = retiring(&other_key().fingerprint().to_string());
    let out = mint_with(&pool, &key(), &retired, "http://127.0.0.1:1")
        .await
        .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
}

/// A row that fails under the very key that sealed it is a row-level fault (tamper, transplant,
/// a sealing bug). No key change heals it, so the user is offered a re-link.
///
/// **Why this bites:** dropping the [`Unopenable::RowFault`] arm sends this row to `UnknownKey`
/// (no retirement is configured), which is a 500.
#[sqlx::test(migrations = "../../migrations")]
async fn tamper_under_the_right_key_mints_not_vaulted(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
    tamper_at(&pool, PRINCIPAL).await;

    let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1")
        .await
        .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
}

/// Bytes that open under the right key but are not UTF-8 were sealed by something holding the
/// key. That is a bug or a compromise, never a refusal.
///
/// **Why this bites:** classifying a decoded-but-invalid payload as a tag failure would land it
/// in `RowFault` (the row carries the configured key's fingerprint), which is `NotVaulted`.
#[sqlx::test(migrations = "../../migrations")]
async fn invalid_utf8_under_the_right_key_is_a_500(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
    let (nonce, ciphertext) = key().encrypt(&[0xff, 0xfe, 0xfd], &aad(PRINCIPAL, FIELD_AT));
    sqlx::query!(
        "UPDATE kb_slack_grant_vault SET at_nonce = $2, at_ciphertext = $3 \
          WHERE slack_principal_id = $1",
        PRINCIPAL,
        &nonce[..],
        ciphertext,
    )
    .execute(&pool)
    .await
    .unwrap();

    let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
    assert!(matches!(out, Err(ApiError::Internal(_))), "got {out:?}");
}

/// A legacy row (NULL fingerprint) gets one on its first successful open.
///
/// **Why this bites:** without the stamp on the cached path the column stays NULL.
#[sqlx::test(migrations = "../../migrations")]
async fn a_legacy_row_is_stamped_on_its_first_cached_open(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "cached-at", Some(3600)).await;
    unstamp(&pool, PRINCIPAL).await;

    let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1")
        .await
        .unwrap();
    assert!(matches!(out, MintOutcome::Token { .. }), "got {out:?}");
    assert_eq!(
        stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
        Some(key().fingerprint().as_bytes()),
    );
}

/// The refresh path stamps a legacy row even when the IdP refuses the grant: the refusal arm
/// commits the stamp, so a dead grant is still classified when the key later rotates.
///
/// **Why this bites:** stamping only in the reseal `UPDATE` (which the refusal never reaches),
/// or not committing on the refusal arm, leaves the column NULL.
#[sqlx::test(migrations = "../../migrations")]
async fn a_legacy_row_is_stamped_on_the_refresh_path_even_when_the_idp_refuses(pool: PgPool) {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(serde_json::json!({ "error": "invalid_grant" })),
        )
        .mount(&idp)
        .await;

    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
    unstamp(&pool, PRINCIPAL).await;

    let out = mint_with(&pool, &key(), &RetiredKeys::default(), &idp.uri())
        .await
        .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
    assert_eq!(
        stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
        Some(key().fingerprint().as_bytes()),
    );
}

/// The reseal after a successful refresh stamps the configured key's fingerprint. The row here
/// is stamped with the WRONG fingerprint but opens, which is also the restamp case.
///
/// **Why this bites:** only when BOTH writers are removed. The early stamp (which runs for any
/// row not already carrying the configured key's fingerprint) and the reseal `UPDATE` each set
/// the column, so dropping either one alone still passes. Dropping both leaves the old key's
/// fingerprint on a row now sealed under the new one.
#[sqlx::test(migrations = "../../migrations")]
async fn a_refresh_reseals_and_stamps_the_configured_key(pool: PgPool) {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let idp = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fresh-at",
            "refresh_token": "rotated-rt",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&idp)
        .await;

    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
    let stale = other_key().fingerprint();
    sqlx::query!(
        "UPDATE kb_slack_grant_vault SET key_fingerprint = $2 WHERE slack_principal_id = $1",
        PRINCIPAL,
        stale.as_bytes(),
    )
    .execute(&pool)
    .await
    .unwrap();

    let out = mint_with(&pool, &key(), &RetiredKeys::default(), &idp.uri())
        .await
        .unwrap();
    match out {
        MintOutcome::Token { access_token, .. } => assert_eq!(access_token, "fresh-at"),
        other => panic!("expected a refreshed token, got {other:?}"),
    }
    assert_eq!(
        stored_fingerprint(&pool, PRINCIPAL).await.as_deref(),
        Some(key().fingerprint().as_bytes()),
    );
}

/// A legacy row that does not open has no evidence of its key: a 500, unless the operator
/// retired `unstamped`.
///
/// **Why this bites:** treating NULL as the configured key would make the first call a
/// `RowFault` refusal; ignoring `unstamped` would make the second a 500.
#[sqlx::test(migrations = "../../migrations")]
async fn an_unopenable_legacy_row_is_a_500_unless_unstamped_is_retired(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store_sealed_by(
        &pool,
        &other_key(),
        profile,
        PRINCIPAL,
        "rt",
        "at",
        Some(3600),
    )
    .await;
    unstamp(&pool, PRINCIPAL).await;

    let out = mint_with(&pool, &key(), &RetiredKeys::default(), "http://127.0.0.1:1").await;
    assert!(matches!(out, Err(ApiError::Internal(_))), "got {out:?}");

    let out = mint_with(&pool, &key(), &retiring("unstamped"), "http://127.0.0.1:1")
        .await
        .unwrap();
    assert_eq!(out, MintOutcome::Refused(LinkRefusal::NotVaulted));
}

/// The startup check reports a mismatch when the configured key sealed none of the recent rows.
///
/// **Why this bites:** each assertion pins one arm: a mismatch under the wrong key, a match
/// under the right one, and a retired key that no longer counts as unknown.
#[sqlx::test(migrations = "../../migrations")]
async fn the_startup_check_flags_a_key_that_sealed_none_of_the_recent_rows(pool: PgPool) {
    let profile = insert_profile(&pool).await;
    store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;
    let cfg =
        |vault_key: VaultKey, retired_vault_keys: RetiredKeys| crate::config::SlackLinkConfig {
            client_id: "c".to_string(),
            hmac_secret: "s".to_string(),
            public_base_url: "https://temper.test".to_string(),
            vault_key,
            retired_vault_keys,
        };

    let wrong = check_configured_key(&pool, &cfg(other_key(), RetiredKeys::default()))
        .await
        .expect("the query runs");
    assert!(wrong.is_mismatch(), "{wrong:?}");
    assert_eq!(wrong.unknown, vec![(key().fingerprint(), 1)]);

    let right = check_configured_key(&pool, &cfg(key(), RetiredKeys::default()))
        .await
        .expect("the query runs");
    assert_eq!(
        right,
        KeyCheck {
            sampled: 1,
            matching: 1,
            unknown: vec![],
        }
    );

    let retired =
        RetiredKeys::parse(&key().fingerprint().to_string(), other_key().fingerprint()).unwrap();
    let rotated = check_configured_key(&pool, &cfg(other_key(), retired))
        .await
        .expect("the query runs");
    assert!(!rotated.is_mismatch(), "{rotated:?}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn take_refresh_token_returns_the_sealed_token_in_plaintext(pool: PgPool) {
    let key = key();
    let profile_id = insert_profile(&pool).await;
    store(
        &pool,
        profile_id,
        "slack:T1:U1",
        "rt-plaintext",
        "at-plaintext",
        Some(3600),
    )
    .await;

    let mut conn = pool.acquire().await.expect("acquire");
    let got = take_refresh_token_locked(&mut conn, &key, "slack:T1:U1")
        .await
        .expect("take");
    assert_eq!(got.as_deref(), Some("rt-plaintext"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn take_refresh_token_returns_none_for_an_unknown_principal(pool: PgPool) {
    let key = key();
    let mut conn = pool.acquire().await.expect("acquire");
    let got = take_refresh_token_locked(&mut conn, &key, "slack:T9:U9")
        .await
        .expect("take");
    assert!(
        got.is_none(),
        "an unvaulted principal must yield None, not an error"
    );
}
