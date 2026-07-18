//! The Slack grant vault's persistence: seal a per-user grant, mint access tokens from it,
//! and read it back for disconnect to destroy. All SQL for T3 lives here; the callback handler
//! dispatches and never touches the database or the cipher.
//!
//! The vault holds each linked Slack user's OWN refresh token — the independent grant family
//! T2's `offline_access` consent minted, never an export of that user's local CLI grant. The RT
//! (and a cached access token) are stored as XChaCha20-Poly1305 ciphertext via
//! [`grant_crypto`](super::grant_crypto); the database never sees plaintext or the key.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::grant_crypto::VaultKey;
use crate::error::{ApiError, ApiResult};
use crate::oauth_client;

/// Refresh a cached access token once it is within this window of expiry. Matches temper-client's
/// `needs_refresh` (`auth.rs`): a token that outlives the mention it is minted for is the point.
const AT_REFRESH_SKEW: Duration = Duration::minutes(5);

/// Fallback access-token lifetime when the IdP omits `expires_in`. Mirrors temper-client's
/// refresh default; Auth0 always sends one, so this only guards a spec-legal omission.
const DEFAULT_AT_TTL_SECS: u64 = 3600;

/// What a mint produced. A typed outcome, not `Option<String>`: "no grant on file" and "the
/// grant was revoked" are different facts the caller (T4) must be able to tell apart and say
/// something specific about, and neither is an error.
///
/// `Debug` is hand-written to REDACT the token: `MintOutcome::Token` wraps a live, presentable
/// access token, and this is the exact value the mention path handles — a stray `?outcome` in a
/// log would otherwise write an act-as-the-human credential to disk.
#[derive(Clone, PartialEq, Eq)]
pub enum MintOutcome {
    /// A valid access token the caller may present as the linked human.
    Token(String),
    /// A vault row exists but is not mintable — explicitly revoked, or its profile deactivated.
    /// Mints nothing. (A live token handed out earlier still survives to its own `exp` — see the
    /// migration's note on honest revocation semantics.)
    Revoked,
    /// No grant is vaulted for this principal — the user linked before T3 shipped, or never
    /// completed a link. The caller should route them back through the link flow.
    NotVaulted,
}

impl std::fmt::Debug for MintOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token(_) => f.write_str("Token(redacted)"),
            Self::Revoked => f.write_str("Revoked"),
            Self::NotVaulted => f.write_str("NotVaulted"),
        }
    }
}

/// A freshly obtained grant to seal, straight off the callback exchange. A params struct rather
/// than a long argument list: five same-domain values whose order (two opaque token strings side
/// by side) is easy to transpose at the call site — the named fields make that impossible.
///
/// `Debug` is hand-written to REDACT the two token fields: this struct carries the plaintext
/// refresh and access tokens, so a derived `Debug` + a stray `?grant` would dump the durable
/// grant to a log.
pub struct NewGrant<'a> {
    pub profile_id: Uuid,
    /// The WHOLE opaque principal (`slack:<team>:<user>`), never split.
    pub slack_principal_id: &'a str,
    /// The refresh token — the durable grant.
    pub refresh_token: &'a str,
    /// The access token from the same exchange, cached so the first mention spends no refresh.
    pub access_token: &'a str,
    /// The exchange's `expires_in`. `None` falls back to `DEFAULT_AT_TTL_SECS`.
    pub access_ttl_secs: Option<u64>,
}

impl std::fmt::Debug for NewGrant<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewGrant")
            .field("profile_id", &self.profile_id)
            .field("slack_principal_id", &self.slack_principal_id)
            .field("refresh_token", &"redacted")
            .field("access_token", &"redacted")
            .field("access_ttl_secs", &self.access_ttl_secs)
            .finish()
    }
}

/// Associated data binding a sealed secret to its principal and field. It is covered by the AEAD
/// tag (authenticated, not encrypted), so a valid ciphertext transplanted into another row or the
/// other field fails to open — see [`grant_crypto`](super::grant_crypto). The NUL separator keeps
/// `principal || field` unambiguous; Slack principals never contain NUL.
fn aad(slack_principal_id: &str, field: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(slack_principal_id.len() + 1 + field.len());
    v.extend_from_slice(slack_principal_id.as_bytes());
    v.push(0);
    v.extend_from_slice(field);
    v
}

/// Field tags for [`aad`] — distinguish the refresh-token column from the access-token column so
/// one cannot be opened in place of the other.
const FIELD_RT: &[u8] = b"rt";
const FIELD_AT: &[u8] = b"at";

/// Seal a freshly obtained grant for its principal. Called at T2's callback seam once the
/// directory row is written.
///
/// Upsert on the principal: re-linking (a fresh consent) replaces the stored grant with the new
/// one and **clears any prior revocation** — a new consent is a new, working grant, so a user who
/// disconnected and reconnected is not left mysteriously revoked. The principal binds to one
/// profile (T2's guarantee), so `profile_id` is re-stamped defensively but does not move.
pub async fn store_grant(pool: &PgPool, key: &VaultKey, grant: NewGrant<'_>) -> ApiResult<()> {
    let (rt_nonce, rt_ciphertext) = key.encrypt(
        grant.refresh_token.as_bytes(),
        &aad(grant.slack_principal_id, FIELD_RT),
    );
    let (at_nonce, at_ciphertext) = key.encrypt(
        grant.access_token.as_bytes(),
        &aad(grant.slack_principal_id, FIELD_AT),
    );
    let ttl = grant.access_ttl_secs.unwrap_or(DEFAULT_AT_TTL_SECS);
    let access_expires_at = Utc::now() + Duration::seconds(ttl as i64);

    sqlx::query!(
        r#"
        INSERT INTO kb_slack_grant_vault
            (id, profile_id, slack_principal_id, key_version,
             rt_nonce, rt_ciphertext, at_nonce, at_ciphertext, access_expires_at, revoked_at)
        VALUES ($1, $2, $3, 1, $4, $5, $6, $7, $8, NULL)
        ON CONFLICT (slack_principal_id) DO UPDATE
            SET profile_id        = EXCLUDED.profile_id,
                key_version       = EXCLUDED.key_version,
                rt_nonce          = EXCLUDED.rt_nonce,
                rt_ciphertext     = EXCLUDED.rt_ciphertext,
                at_nonce          = EXCLUDED.at_nonce,
                at_ciphertext     = EXCLUDED.at_ciphertext,
                access_expires_at = EXCLUDED.access_expires_at,
                revoked_at        = NULL,
                updated_at        = now()
        "#,
        Uuid::now_v7(),
        grant.profile_id,
        grant.slack_principal_id,
        &rt_nonce[..],
        rt_ciphertext,
        &at_nonce[..],
        at_ciphertext,
        access_expires_at,
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Mint a fresh access token for `slack_principal_id`, refreshing the grant only when the cached
/// token is near expiry.
///
/// The whole thing runs in one transaction under a `SELECT ... FOR UPDATE OF v` row lock (locking
/// the vault row only, not the joined profile). The lock serializes CONCURRENT mints of the same
/// principal: Auth0 rotates the refresh token on every refresh, so two mentions arriving at once
/// must not both spend the same stored RT — the second blocks, then reads (under READ COMMITTED's
/// EvalPlanQual re-read) the RT the first already rotated in, and finds the cache fresh so it does
/// not refresh at all.
///
/// **What the lock does NOT close** (documented, not solved here): the *dual-write* window. The RT
/// rotation is an IdP side effect that is not part of this transaction, so if the process dies or
/// the COMMIT fails AFTER Auth0 returns a rotated RT but BEFORE the `UPDATE` commits, the row keeps
/// the now-dead RT and the next mint trips reuse-detection, bricking the grant until the user
/// re-links. No row lock can make an external HTTP effect atomic with a local commit. Two
/// mitigations belong at the deployment layer, not here: enable Auth0 refresh-token rotation
/// **leeway** (a grace window that tolerates a brief RT reuse), and treat a bricked grant as a
/// re-link prompt. See the T3 spec's "grant vault" section.
///
/// A profile that has been **deactivated** (`kb_profiles.is_active = false`) mints nothing — the
/// deactivation kill-switch reaches the vault here, mirroring the link path's rejection of a
/// deactivated profile. It is reported as [`MintOutcome::Revoked`] (not mintable).
///
/// `token_url` / `client_id` come from `link_provider::derive` at the call site — the same public
/// client that minted the grant. They are NOT stored on the row: the instance's config is
/// authoritative, and a config change that invalidates old grants invalidates them at the IdP too.
///
/// AUTHORIZATION: this function enforces none — it mints for whatever principal it is handed. Its
/// caller (T4's mention path) MUST derive `slack_principal_id` from an HMAC-verified server-to-
/// server request whose principal comes from Slack's own verified event, never a client-supplied
/// field. Naming a principal must not be sufficient to mint its token.
pub async fn mint_access_token(
    pool: &PgPool,
    key: &VaultKey,
    token_url: &str,
    client_id: &str,
    slack_principal_id: &str,
) -> ApiResult<MintOutcome> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query!(
        r#"
        SELECT v.rt_nonce, v.rt_ciphertext, v.at_nonce, v.at_ciphertext,
               v.access_expires_at, v.revoked_at, p.is_active
          FROM kb_slack_grant_vault v
          JOIN kb_profiles p ON p.id = v.profile_id
         WHERE v.slack_principal_id = $1
         FOR UPDATE OF v
        "#,
        slack_principal_id,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Ok(MintOutcome::NotVaulted);
    };
    // Not-mintable checks first, before any cached token is decrypted or the RT is spent: an
    // explicit revocation, OR a deactivated profile (the kill-switch must reach the vault).
    if row.revoked_at.is_some() || !row.is_active {
        return Ok(MintOutcome::Revoked);
    }

    // Cached access token still comfortably valid? Hand it back — no refresh, no RT rotation.
    if let (Some(at_nonce), Some(at_ciphertext), Some(expires_at)) =
        (&row.at_nonce, &row.at_ciphertext, row.access_expires_at)
    {
        if expires_at > Utc::now() + AT_REFRESH_SKEW {
            let token = decrypt_to_string(
                key,
                at_nonce,
                at_ciphertext,
                &aad(slack_principal_id, FIELD_AT),
            )?;
            tx.commit().await?;
            return Ok(MintOutcome::Token(token));
        }
    }

    // The cached token is gone or stale: spend the RT, then rotate everything the response gave.
    let refresh_token = decrypt_to_string(
        key,
        &row.rt_nonce,
        &row.rt_ciphertext,
        &aad(slack_principal_id, FIELD_RT),
    )?;
    let tokens = oauth_client::refresh_grant(token_url, client_id, &refresh_token).await?;

    // Auth0 rotates, so the response's RT supersedes the spent one. A provider that does not
    // rotate omits it (RFC 6749 §6: keep the existing RT); keep the one we hold in that case
    // rather than blanking the grant. NOTE: against a rotating IdP a 200-without-RT is a smell —
    // the leeway mitigation above is what keeps re-storing the old RT from bricking the family.
    let new_refresh = tokens.refresh_token.as_deref().unwrap_or(&refresh_token);
    let (rt_nonce, rt_ciphertext) =
        key.encrypt(new_refresh.as_bytes(), &aad(slack_principal_id, FIELD_RT));
    let (at_nonce, at_ciphertext) = key.encrypt(
        tokens.access_token.as_bytes(),
        &aad(slack_principal_id, FIELD_AT),
    );
    let ttl = tokens.expires_in.unwrap_or(DEFAULT_AT_TTL_SECS);
    let access_expires_at = Utc::now() + Duration::seconds(ttl as i64);

    sqlx::query!(
        r#"
        UPDATE kb_slack_grant_vault
           SET rt_nonce = $2, rt_ciphertext = $3,
               at_nonce = $4, at_ciphertext = $5,
               access_expires_at = $6, updated_at = now()
         WHERE slack_principal_id = $1
        "#,
        slack_principal_id,
        &rt_nonce[..],
        rt_ciphertext,
        &at_nonce[..],
        at_ciphertext,
        access_expires_at,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(MintOutcome::Token(tokens.access_token))
}

/// Read and decrypt the stored refresh token, for the disconnect path only.
///
/// Takes a `&mut PgConnection` so the caller can run this inside the same
/// transaction as the deletes. Locks the row (`FOR UPDATE`) so a concurrent
/// `mint_access_token` cannot rotate the RT out from under the revoke.
///
/// Returns `Ok(None)` when the principal has no vault row — a pre-T3 link, or
/// an already-disconnected principal. That is not an error: disconnect is
/// idempotent, and a missing grant simply means there is nothing to revoke.
///
/// Deliberately ignores `revoked_at`: a soft-revoked row still holds live
/// ciphertext, and disconnect's job is to destroy it.
pub async fn take_refresh_token_for_disconnect(
    tx: &mut sqlx::PgConnection,
    key: &VaultKey,
    slack_principal_id: &str,
) -> ApiResult<Option<String>> {
    let row = sqlx::query!(
        r#"
        SELECT rt_nonce, rt_ciphertext
          FROM kb_slack_grant_vault
         WHERE slack_principal_id = $1
         FOR UPDATE
        "#,
        slack_principal_id
    )
    .fetch_optional(&mut *tx)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let rt = decrypt_to_string(
        key,
        &row.rt_nonce,
        &row.rt_ciphertext,
        &aad(slack_principal_id, FIELD_RT),
    )?;
    Ok(Some(rt))
}

/// Open a stored `(nonce, ciphertext)` pair to a UTF-8 token string, under the `aad` it was sealed
/// with. A decrypt failure means the key changed, the row was tampered, or the ciphertext was
/// transplanted from another principal/field (AAD mismatch); a non-UTF-8 payload means the same
/// (tokens are ASCII). Either way the grant is unusable — surface an internal error rather than a
/// token we can't trust.
fn decrypt_to_string(
    key: &VaultKey,
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> ApiResult<String> {
    let bytes = key
        .decrypt(nonce, ciphertext, aad)
        .map_err(|e| ApiError::Internal(format!("vault: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|_| ApiError::Internal("vault: decrypted grant was not valid UTF-8".to_string()))
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::super::grant_crypto::VaultKey;
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    fn key() -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([3u8; 32])).unwrap()
    }

    /// Minimal profile insert — this suite tests the vault, not provisioning. The handle is the
    /// FULL id: two UUIDv7s minted in the same millisecond share their first bytes, so a truncated
    /// handle collides on `kb_profiles_handle_key` when a test inserts two profiles.
    async fn insert_profile(pool: &PgPool) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
            id,
            format!("user-{id}"),
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// Seal a grant under the test key. Wraps the [`NewGrant`] construction the tests would
    /// otherwise repeat.
    async fn store(
        pool: &PgPool,
        profile: Uuid,
        principal: &str,
        rt: &str,
        at: &str,
        ttl: Option<u64>,
    ) {
        store_grant(
            pool,
            &key(),
            NewGrant {
                profile_id: profile,
                slack_principal_id: principal,
                refresh_token: rt,
                access_token: at,
                access_ttl_secs: ttl,
            },
        )
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
            "http://127.0.0.1:1/unused",
            "client",
            PRINCIPAL,
        )
        .await
        .unwrap();
        assert_eq!(out, MintOutcome::Token("cached-access-token".to_string()));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_reports_not_vaulted_for_an_unknown_principal(pool: PgPool) {
        let out = mint_access_token(&pool, &key(), "http://127.0.0.1:1", "c", PRINCIPAL)
            .await
            .unwrap();
        assert_eq!(out, MintOutcome::NotVaulted);
    }

    /// After revoke, mint reports Revoked and never reaches the (unreachable) token endpoint.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_reports_revoked_and_does_not_refresh(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        // Store with an already-expired cached AT so, absent the revocation, mint WOULD refresh.
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(0)).await;
        // Flip the flag directly — the write-side `revoke` fn is gone (disconnect now deletes
        // the row instead), but mint must still honour a row flagged before that change.
        sqlx::query!(
            "UPDATE kb_slack_grant_vault SET revoked_at = now() WHERE slack_principal_id = $1",
            PRINCIPAL,
        )
        .execute(&pool)
        .await
        .unwrap();

        let out = mint_access_token(&pool, &key(), "http://127.0.0.1:1/unused", "c", PRINCIPAL)
            .await
            .unwrap();
        assert_eq!(
            out,
            MintOutcome::Revoked,
            "a revoked grant must mint nothing and must not spend the RT",
        );
    }

    /// The deactivation kill-switch reaches the vault: a grant whose profile is deactivated mints
    /// nothing, even with a still-valid cached token and without ever being explicitly revoked.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_refuses_a_deactivated_profile(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        // A comfortably-valid cached AT — absent the is_active gate, mint would hand it straight back.
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
            "UPDATE kb_profiles SET is_active = false WHERE id = $1",
            profile
        )
        .execute(&pool)
        .await
        .unwrap();

        let out = mint_access_token(&pool, &key(), "http://127.0.0.1:1/unused", "c", PRINCIPAL)
            .await
            .unwrap();
        assert_eq!(
            out,
            MintOutcome::Revoked,
            "a deactivated profile must not mint — the kill-switch must reach the vault",
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

        let result =
            mint_access_token(&pool, &key(), "http://127.0.0.1:1/unused", "c", PRINCIPAL_B).await;
        assert!(
            result.is_err(),
            "a ciphertext sealed for principal A must not decrypt under principal B — the AAD \
             binding must fail the tag rather than yield A's refresh token to B",
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
        let out = mint_access_token(&pool, &key(), "http://127.0.0.1:1/unused", "c", PRINCIPAL)
            .await
            .unwrap();
        assert_eq!(out, MintOutcome::Token("at2-fresh".to_string()));
    }

    /// The principal is the key, WHOLE. A different principal must not read another's grant.
    #[sqlx::test(migrations = "../../migrations")]
    async fn mint_does_not_match_a_different_principal(pool: PgPool) {
        let profile = insert_profile(&pool).await;
        store(&pool, profile, PRINCIPAL, "rt", "at", Some(3600)).await;

        let out = mint_access_token(
            &pool,
            &key(),
            "http://127.0.0.1:1",
            "c",
            "slack:T0BHAHEN79C:UOTHER",
        )
        .await
        .unwrap();
        assert_eq!(out, MintOutcome::NotVaulted);
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
        let got = take_refresh_token_for_disconnect(&mut conn, &key, "slack:T1:U1")
            .await
            .expect("take");
        assert_eq!(got.as_deref(), Some("rt-plaintext"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn take_refresh_token_returns_none_for_an_unknown_principal(pool: PgPool) {
        let key = key();
        let mut conn = pool.acquire().await.expect("acquire");
        let got = take_refresh_token_for_disconnect(&mut conn, &key, "slack:T9:U9")
            .await
            .expect("take");
        assert!(
            got.is_none(),
            "an unvaulted principal must yield None, not an error"
        );
    }
}
