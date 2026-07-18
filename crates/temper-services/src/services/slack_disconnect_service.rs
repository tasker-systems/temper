//! Disconnect — unbind a Slack principal from a temper profile.
//!
//! The single chokepoint for every disconnect surface. Ordering is forced, not
//! chosen: the IdP's revocation endpoint takes the refresh token as a body
//! parameter, so the revoke must happen while the ciphertext still exists.
//!
//! The revoke is deliberately **best-effort**. Disconnect is the only unbind
//! lever in the system — and the remediation path for a mis-bound principal —
//! so gating it on third-party availability would be strictly worse than the
//! residual risk it closes. On failure we destroy the local copy anyway and log
//! a structured warning; we never persist the token to retry later, because
//! that would preserve the exact secret the user asked us to destroy.

use sqlx::PgPool;

use super::grant_crypto::VaultKey;
use super::slack_grant_vault_service;
use super::slack_link_service::SLACK_AUTH_PROVIDER;
use crate::auth_config::AuthMode;
use crate::error::ApiResult;
use crate::oauth_client;

/// What a disconnect actually did. Every field is an observation, not a promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectOutcome {
    /// Whether an identity row existed and was removed.
    pub was_linked: bool,
    /// Whether a vault row existed and was destroyed.
    pub grant_deleted: bool,
    /// How many link intents were swept for this principal.
    pub intents_deleted: i64,
    /// Whether the IdP acknowledged the revocation. `false` is not a failure of
    /// the disconnect — see the module docs.
    pub idp_revoked: bool,
}

/// Inputs for a disconnect. A params struct because this crosses five
/// domain-related values and the codebase forbids growing the arg list.
#[derive(Debug)]
pub struct DisconnectRequest<'a> {
    pub slack_principal_id: &'a str,
    pub key: &'a VaultKey,
    pub mode: AuthMode,
    pub revoke_url: String,
    pub client_id: &'a str,
}

/// Unbind a Slack principal: revoke the grant, then delete identity, secret and
/// intents in one transaction.
///
/// Idempotent — disconnecting an unlinked principal succeeds quietly with every
/// outcome flag false.
pub async fn disconnect_slack_principal(
    pool: &PgPool,
    req: DisconnectRequest<'_>,
) -> ApiResult<DisconnectOutcome> {
    let mut tx = pool.begin().await?;

    // 1. Read + decrypt the RT while it still exists (locks the vault row).
    let refresh_token = slack_grant_vault_service::take_refresh_token_for_disconnect(
        &mut tx,
        req.key,
        req.slack_principal_id,
    )
    .await?;

    // 2. Revoke. Best-effort in ExternalIdp mode; real and atomic in AS mode.
    let idp_revoked = match (&refresh_token, req.mode) {
        (None, _) => false,
        (Some(rt), AuthMode::TemperAs) => {
            // The AS issued this token and stores it locally, so revocation is a
            // row update in THIS transaction — no network, no failure mode.
            revoke_as_refresh_token(&mut tx, rt).await?
        }
        (Some(rt), AuthMode::ExternalIdp) => {
            match oauth_client::revoke_grant(&req.revoke_url, req.client_id, rt).await {
                Ok(()) => true,
                Err(e) => {
                    // Principal + error only. Never the token.
                    tracing::warn!(
                        principal = %req.slack_principal_id,
                        error = %e,
                        "slack disconnect: IdP revocation failed; destroying the local grant \
                         anyway. The grant may remain live at the IdP until it expires — \
                         revoke it out-of-band if that matters."
                    );
                    false
                }
            }
        }
    };

    // 3. Destroy the secret.
    let grant_deleted = sqlx::query!(
        "DELETE FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        req.slack_principal_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    // 4. Destroy the identity binding.
    let was_linked = sqlx::query!(
        r#"
        DELETE FROM kb_profile_auth_links
         WHERE auth_provider = $1
           AND auth_provider_user_id = $2
        "#,
        SLACK_AUTH_PROVIDER,
        req.slack_principal_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    // 5. Sweep intents.
    //
    // Load-bearing, and NOT hygiene. The link design closes URL-theft with two
    // guarantees: D9 never issues a URL to a linked user, and rebind is refused.
    // A disconnect removes BOTH at once — so any intent minted before it and
    // still inside its TTL becomes a live, consumable *first-link* URL for a
    // now-unlinked principal. Leaving these rows reopens the hole disconnect is
    // supposed to be safe against.
    let intents_deleted = sqlx::query!(
        "DELETE FROM kb_slack_link_intents WHERE slack_principal_id = $1",
        req.slack_principal_id
    )
    .execute(&mut *tx)
    .await?
    .rows_affected() as i64;

    tx.commit().await?;

    tracing::info!(
        principal = %req.slack_principal_id,
        was_linked,
        grant_deleted,
        intents_deleted,
        idp_revoked,
        "slack disconnect completed"
    );

    Ok(DisconnectOutcome {
        was_linked,
        grant_deleted,
        intents_deleted,
        idp_revoked,
    })
}

/// Revoke a temper-AS refresh token locally, in the caller's transaction.
///
/// The AS stores only `sha256(token)` as lowercase hex (`packages/temper-cloud/
/// src/oauth/mint.ts:85` — `createHash("sha256").update(t).digest("hex")`), so
/// we reproduce that digest and flip `revoked_at`. Idempotent, matching the
/// TypeScript `revokeRefreshToken` (`flow.ts:179`).
async fn revoke_as_refresh_token(
    tx: &mut sqlx::PgConnection,
    refresh_token: &str,
) -> ApiResult<bool> {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(refresh_token.as_bytes());
    let token_hash = format!("{digest:x}");

    let affected = sqlx::query!(
        r#"
        UPDATE kb_oauth_refresh_tokens
           SET revoked_at = now()
         WHERE token_hash = $1
           AND revoked_at IS NULL
        "#,
        token_hash
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    Ok(affected > 0)
}

/// Sweep link intents that can no longer serve a purpose.
///
/// An intent is dead once it has expired or been consumed — a consumed row's
/// nonce is single-use and already burnt. Until this existed nothing ever
/// deleted from `kb_slack_link_intents`, so every principal that ever linked
/// left its PKCE verifier on disk indefinitely.
///
/// Live rows (unconsumed and unexpired) are spared: they are challenges a user
/// may still be about to click.
pub async fn reap_expired_intents(pool: &PgPool) -> ApiResult<i64> {
    let swept = sqlx::query!(
        r#"
        DELETE FROM kb_slack_link_intents
         WHERE consumed_at IS NOT NULL
            OR expires_at <= now()
        "#
    )
    .execute(pool)
    .await?
    .rows_affected() as i64;

    if swept > 0 {
        tracing::info!(swept, "slack link intents reaped");
    }
    Ok(swept)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use sqlx::PgPool;
    use uuid::Uuid;

    // `key()` and `insert_profile()` exist in slack_grant_vault_service's test
    // module, but a `#[cfg(test)] mod tests` is private to its own module — they
    // are NOT reachable from here. Redefined locally, matching that module's
    // shape exactly (same key bytes, same full-UUID handle rationale).

    fn key() -> VaultKey {
        VaultKey::from_base64(&STANDARD.encode([3u8; 32])).unwrap()
    }

    /// Minimal profile insert. The handle is the FULL id: two UUIDv7s minted in
    /// the same millisecond share leading bytes, so a truncated handle collides
    /// on `kb_profiles_handle_key`.
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn disconnecting_an_unlinked_principal_is_a_quiet_no_op(pool: PgPool) {
        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: "slack:T0:UNEVER",
                key: &key(),
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/unused".to_string(),
                client_id: "c",
            },
        )
        .await
        .expect("idempotent disconnect must not error");

        assert!(!out.was_linked);
        assert!(!out.grant_deleted);
        assert_eq!(out.intents_deleted, 0);
        assert!(!out.idp_revoked);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn disconnect_deletes_link_grant_and_intents_together(pool: PgPool) {
        let principal = "slack:T1:U1";
        let key = key();
        let profile_id = insert_profile(&pool).await;

        crate::services::slack_link_service::link_slack_principal(&pool, profile_id, principal)
            .await
            .expect("link");
        crate::services::slack_grant_vault_service::store_grant(
            &pool,
            &key,
            crate::services::slack_grant_vault_service::NewGrant {
                profile_id,
                slack_principal_id: principal,
                refresh_token: "rt",
                access_token: "at",
                access_ttl_secs: Some(3600),
            },
        )
        .await
        .expect("store");
        crate::services::slack_link_service::create_intent(
            &pool,
            principal,
            "verifier",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("intent");

        // Unreachable revoke URL: the IdP call must fail and must NOT block the unbind.
        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key,
                mode: AuthMode::ExternalIdp,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "c",
            },
        )
        .await
        .expect("a failed IdP revoke must not fail the disconnect");

        assert!(out.was_linked);
        assert!(out.grant_deleted);
        assert_eq!(out.intents_deleted, 1);
        assert!(
            !out.idp_revoked,
            "the unreachable IdP must report not-revoked"
        );

        let links: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider = 'slack' AND auth_provider_user_id = $1",
        )
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(links, 0, "the identity row must be gone");

        let grants: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1",
        )
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(grants, 0, "the sealed grant must be destroyed, not flagged");

        let intents: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_slack_link_intents WHERE slack_principal_id = $1",
        )
        .bind(principal)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(intents, 0, "live intents must not survive a disconnect");

        // The profile itself is untouched.
        let alive: bool = sqlx::query_scalar("SELECT is_active FROM kb_profiles WHERE id = $1")
            .bind(profile_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(alive, "disconnect is not deactivation");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn disconnecting_twice_is_not_an_error(pool: PgPool) {
        let principal = "slack:T2:U2";
        let key = key();
        let profile_id = insert_profile(&pool).await;
        crate::services::slack_link_service::link_slack_principal(&pool, profile_id, principal)
            .await
            .expect("link");

        let req = || DisconnectRequest {
            slack_principal_id: principal,
            key: &key,
            mode: AuthMode::ExternalIdp,
            revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
            client_id: "c",
        };

        let first = disconnect_slack_principal(&pool, req())
            .await
            .expect("first");
        assert!(first.was_linked);
        let second = disconnect_slack_principal(&pool, req())
            .await
            .expect("second");
        assert!(!second.was_linked, "the second disconnect is a quiet no-op");
    }

    /// The cross-language contract, pinned.
    ///
    /// `kb_oauth_refresh_tokens` rows are written by TypeScript
    /// (`packages/temper-cloud/src/oauth/mint.ts:85` — `createHash("sha256")
    /// .update(t).digest("hex")`), and revoked here by Rust. Nothing in the type
    /// system connects the two: if these digests ever disagree, the AS-mode
    /// revoke silently updates ZERO rows and reports success, leaving a live
    /// grant behind with no error anywhere.
    ///
    /// The expected value below was produced by the actual writer:
    ///   node -e 'const{createHash}=require("crypto");
    ///            console.log(createHash("sha256").update("as-refresh-token-sample")
    ///                        .digest("hex"))'
    /// Regenerate it the same way if this ever needs to change.
    #[test]
    fn the_as_token_hash_matches_what_typescript_writes() {
        use sha2::{Digest, Sha256};

        let digest = Sha256::digest(b"as-refresh-token-sample");
        assert_eq!(
            format!("{digest:x}"),
            "9d16e5d809978fbc29ae240d1b95273fc1ff0de968d8e4f98cadfa0b5802e199",
            "Rust's digest must equal Node's sha256 hex, or AS-mode revocation \
             matches no row and fails silently"
        );
    }

    /// AS mode revokes locally, in the same transaction — no network, no
    /// best-effort. This is why self-hosted gets strictly stronger semantics
    /// than the Auth0 path, so it must actually be exercised.
    #[sqlx::test(migrations = "../../migrations")]
    async fn as_mode_revokes_the_refresh_token_row_in_transaction(pool: PgPool) {
        let principal = "slack:T3:U3";
        let key = key();
        let profile_id = insert_profile(&pool).await;
        let refresh_token = "as-refresh-token-sample";

        crate::services::slack_link_service::link_slack_principal(&pool, profile_id, principal)
            .await
            .expect("link");
        crate::services::slack_grant_vault_service::store_grant(
            &pool,
            &key,
            crate::services::slack_grant_vault_service::NewGrant {
                profile_id,
                slack_principal_id: principal,
                refresh_token,
                access_token: "at",
                access_ttl_secs: Some(3600),
            },
        )
        .await
        .expect("store");

        // The AS's own row for that token, as the TypeScript writer would leave it.
        sqlx::query(
            r#"
            INSERT INTO kb_oauth_refresh_tokens (token_hash, client_id, claims, expires_at)
            VALUES ($1, $2, '{}'::jsonb, now() + interval '30 days')
            "#,
        )
        .bind("9d16e5d809978fbc29ae240d1b95273fc1ff0de968d8e4f98cadfa0b5802e199")
        .bind("slack-link-client")
        .execute(&pool)
        .await
        .expect("seed AS refresh token");

        // An unreachable revoke_url: if AS mode wrongly took the HTTP path, the
        // call would fail and idp_revoked would be false.
        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: principal,
                key: &key,
                mode: AuthMode::TemperAs,
                revoke_url: "http://127.0.0.1:1/oauth/revoke".to_string(),
                client_id: "slack-link-client",
            },
        )
        .await
        .expect("disconnect");

        assert!(
            out.idp_revoked,
            "AS mode must revoke locally without touching the network"
        );

        let revoked_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT revoked_at FROM kb_oauth_refresh_tokens WHERE token_hash = $1",
        )
        .bind("9d16e5d809978fbc29ae240d1b95273fc1ff0de968d8e4f98cadfa0b5802e199")
        .fetch_one(&pool)
        .await
        .expect("read back");
        assert!(
            revoked_at.is_some(),
            "the AS row must be marked revoked, asserted on the row not the return value"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_reaper_sweeps_expired_and_consumed_intents_but_spares_live_ones(pool: PgPool) {
        use crate::services::slack_link_service::create_intent;

        // Live — must survive.
        create_intent(
            &pool,
            "slack:T1:ULIVE",
            "v-live",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("live intent");

        // Expired — must be swept.
        create_intent(
            &pool,
            "slack:T1:UEXP",
            "v-exp",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("expiring intent");
        sqlx::query("UPDATE kb_slack_link_intents SET expires_at = now() - interval '1 hour' WHERE slack_principal_id = $1")
            .bind("slack:T1:UEXP")
            .execute(&pool)
            .await
            .unwrap();

        // Consumed but not yet expired — must be swept (its purpose is spent).
        create_intent(
            &pool,
            "slack:T1:UUSED",
            "v-used",
            std::time::Duration::from_secs(900),
        )
        .await
        .expect("consumed intent");
        sqlx::query(
            "UPDATE kb_slack_link_intents SET consumed_at = now() WHERE slack_principal_id = $1",
        )
        .bind("slack:T1:UUSED")
        .execute(&pool)
        .await
        .unwrap();

        let swept = reap_expired_intents(&pool).await.expect("reap");
        assert_eq!(swept, 2, "expired and consumed rows are swept");

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT slack_principal_id FROM kb_slack_link_intents")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec!["slack:T1:ULIVE".to_string()]);
    }
}
