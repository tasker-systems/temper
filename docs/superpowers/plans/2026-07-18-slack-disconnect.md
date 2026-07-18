# Slack Disconnect Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a linked Slack user unbind their Slack principal from their temper profile — deleting the identity row, the encrypted grant, and any live link intents — via a self-serve CLI command and an admin CLI command.

**Architecture:** One service function, `disconnect_slack_principal`, is the single chokepoint. It decrypts the stored refresh token, makes a **best-effort, non-fatal** revocation attempt at the IdP (HTTP to Auth0 in `ExternalIdp` mode; a local in-transaction UPDATE in `TemperAs` mode), then deletes three rows in one transaction: the vault row, the auth-link row, and every intent row for that principal. Two thin HTTP surfaces dispatch to it — a self-serve `DELETE /api/auth/slack/link/me` deriving the principal from the caller's own profile, and an admin `POST /api/admin/slack/links/disconnect` taking an explicit principal behind an `is_system_admin` gate. A separate cron reaps expired/consumed intents globally.

**Tech Stack:** Rust (axum, sqlx, reqwest), clap (CLI), utoipa (OpenAPI), Vercel Cron.

## Global Constraints

- **The Slack principal is opaque and has 2–4 segments.** Store, compare, and pass it **whole**. **Never** `split(':')` it. Validate only via the existing `validate_slack_principal` (`crates/temper-api/src/handlers/slack_link.rs:133`): non-empty, ≤128 bytes, `slack:` prefix.
- **This work needs no migration** — disconnect and the reaper are pure DML against existing tables. That is an observation about the design, **not** a prohibition. If a task genuinely requires schema change, add one; just sequence it correctly rather than avoiding it.

  **If you do add a migration**, follow the resequence playbook before choosing a number:
  1. `ls migrations/ | tail -3` — the highest on `main` as of this plan is `20260717000030_slack_grant_vault.sql`.
  2. Check what unmerged sibling branches are holding. As of 2026-07-18: `origin/jct/admin-event-sink-task5-grant-chokepoint` holds `20260718000010_admin_grant_fns.sql` (it already resequenced off the `…000030` slot, so that collision is resolved), and `origin/jct/ws7-invocation-envelope-authorship` holds `20260618000001_temper_next_invocation_envelope.sql`.
  3. **Read production's `_sqlx_migrations` before assuming local order is authoritative** — a version that is already applied in prod cannot be renamed.
  4. Leave a gap (take `20260718000030`+, not `…000020`) so a concurrent sibling session has room.
  5. Keep it **additive-only** — `main` auto-deploys, and a `DROP FUNCTION` breaks migrate-ahead-of-deploy skew.
- **No ledger/audit event.** Deliberately deferred to task `019f75ec-f82f-73f1-b038-81993e822f5a`, which lands after admin-event-sink Task 5. Use `tracing` for observability here.
- **Revoke-then-delete ordering is forced, not preferred.** Auth0's `/oauth/revoke` takes the refresh token itself as a body parameter. You cannot revoke a token you have already deleted.
- **The IdP revoke is non-fatal.** Its failure must never block the local unbind. Disconnect is the only unbind lever in the system, and gating it on third-party uptime would also gate the remediation path for the URL-theft case the design acknowledges.
- **Never persist the refresh token past the disconnect.** No outbox, no retry queue, no "revoke pending" row. Any such record would preserve the exact secret the user asked to destroy. On revoke failure: log a structured warning (principal + status, **never** the token) and proceed with deletion.
- **Never log a token, ciphertext, or key.** Match the existing redacting `Debug` discipline (`MintOutcome`, `NewGrant`, `SlackLinkConfig`).
- **Auth before writes.** The `is_system_admin` check goes at the top of the admin handler. The gated router admits everyone under `access_mode='open'` — see the load-bearing warning at `crates/temper-api/src/routes.rs:168-169`. The router is not an admin gate.
- **`output::hint` writes to stdout and corrupts `--format json`.** New commands emit caveats via `output::warning` (stderr). Task 11 fixes the existing callers.
- **Honest revocation semantics, stated in user-facing copy:** disconnect stops **future** mints; an already-issued access token survives to its own `exp` because JWKS validation consults no revocation list. Do not imply instant cutoff.

---

## File Structure

**Create:**
- `crates/temper-services/src/services/slack_disconnect_service.rs` — the chokepoint: revoke-then-delete, plus the intents reaper.
- `crates/temper-api/src/handlers/slack_disconnect.rs` — the two handlers.
- `crates/temper-client/src/slack.rs` — the `SlackClient` sub-client (net-new; no Slack CLI/client surface exists today).
- `crates/temper-cli/src/commands/slack.rs` — self-serve command.
- `crates/temper-cli/src/commands/admin_slack.rs` — admin command.

**Modify:**
- `crates/temper-services/src/oauth_client.rs` — add `revoke_grant`.
- `crates/temper-services/src/services/slack_grant_vault_service.rs` — expose what the disconnect service needs; drop the dead `revoke`.
- `crates/temper-services/src/services/mod.rs`, `crates/temper-api/src/handlers/mod.rs` — module registration.
- `crates/temper-core/src/types/slack.rs` — response DTO (new file in an existing module tree).
- `crates/temper-api/src/routes.rs` — three routes.
- `crates/temper-api/src/handlers/embed.rs` — `require_dispatch_secret` → `pub(crate)`.
- `crates/temper-cli/src/cli.rs`, `main.rs`, `commands/mod.rs` — command wiring.
- `crates/temper-client/src/lib.rs` — sub-client accessor.
- `vercel.json` — reaper cron + route.
- `tests/e2e/tests/common/mod.rs` — `run_temper_cli_with_token`.
- `tests/e2e/tests/slack_link_test.rs` — disconnect e2e tests.
- `docs/guides/slack-setup.md` — disconnect section.
- `openapi.json`, `clients/temper-rb/lib/temper/generated/**`, `clients/temper-ts/src/generated/schema.ts` — regenerated artifacts.

---

## Task 1: `oauth_client::revoke_grant`

**Files:**
- Modify: `crates/temper-services/src/oauth_client.rs`

**Interfaces:**
- Consumes: `temper_auth::TokenResponse`, `ApiResult`, the module-private `static HTTP: LazyLock<reqwest::Client>` (`oauth_client.rs:18`).
- Produces: `pub async fn revoke_grant(revoke_url: &str, client_id: &str, refresh_token: &str) -> ApiResult<()>`

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/temper-services/src/oauth_client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn revoke_posts_the_token_and_client_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/oauth/revoke", server.uri());
        revoke_grant(&url, "test-client", "rt-value")
            .await
            .expect("revoke should succeed on 200");
    }

    #[tokio::test]
    async fn revoke_surfaces_a_non_2xx_as_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth/revoke"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        let url = format!("{}/oauth/revoke", server.uri());
        let err = revoke_grant(&url, "test-client", "rt-value")
            .await
            .expect_err("a 400 must be an error");
        assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
    }
}
```

If `wiremock` is not already a dev-dependency of temper-services, add it. Check first:

```bash
grep -n "wiremock" crates/temper-services/Cargo.toml
```

If absent, add under `[dev-dependencies]` using the workspace pin:

```toml
wiremock = { workspace = true }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-services revoke_posts_the_token_and_client_id`
Expected: FAIL — `cannot find function 'revoke_grant' in this scope`

- [ ] **Step 3: Write minimal implementation**

Add to `crates/temper-services/src/oauth_client.rs`, immediately after `refresh_grant`. This mirrors `refresh_grant`'s error mapping exactly, with two differences: there is no response body to decode, and the timeout is 5s because no DB lock is held across this call.

```rust
/// Best-effort revocation of a refresh-token grant at an external IdP.
///
/// Callers MUST treat a failure as non-fatal — see the disconnect service. The
/// token is a body parameter, which is why revocation has to happen *before*
/// the stored ciphertext is deleted.
///
/// Auth0's revocation endpoint returns 200 with an empty body; there is nothing
/// to decode. A public client (no secret) sends only `client_id`.
pub async fn revoke_grant(
    revoke_url: &str,
    client_id: &str,
    refresh_token: &str,
) -> ApiResult<()> {
    let res = HTTP
        .post(revoke_url)
        .timeout(Duration::from_secs(5))
        .form(&[
            ("client_id", client_id),
            ("token", refresh_token),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("token revoke transport failure: {e}")))?;

    let status = res.status();
    if !status.is_success() {
        // Status only — the body may echo request parameters.
        tracing::warn!(%status, "token revoke returned a non-success status");
        return Err(ApiError::Unauthorized("revoke grant failed".to_string()));
    }

    Ok(())
}
```

Confirm `std::time::Duration` is already imported in this file (`refresh_grant` uses it at `:76`). If the import is function-local, add `use std::time::Duration;` at module scope.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services -E 'test(revoke_posts_the_token_and_client_id) + test(revoke_surfaces_a_non_2xx_as_unauthorized)'`
Expected: PASS, 2 tests

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/oauth_client.rs crates/temper-services/Cargo.toml
git commit -m "feat(slack): add oauth_client::revoke_grant for best-effort IdP revocation"
```

---

## Task 2: Expose the vault internals the disconnect service needs

The disconnect service must decrypt the stored refresh token before deleting the row. `aad`, `decrypt_to_string`, `FIELD_RT`, and `FIELD_AT` are all **private** in `slack_grant_vault_service.rs`. Rather than widening four private items across a module boundary, add the read to the vault service itself — it is the module that owns the ciphertext.

**Files:**
- Modify: `crates/temper-services/src/services/slack_grant_vault_service.rs`

**Interfaces:**
- Consumes: private `aad`, `decrypt_to_string`, `FIELD_RT`; `VaultKey`; `ApiResult`.
- Produces: `pub async fn take_refresh_token_for_disconnect(tx: &mut sqlx::PgConnection, key: &VaultKey, slack_principal_id: &str) -> ApiResult<Option<String>>`

- [ ] **Step 1: Write the failing test**

Add inside the existing `#[cfg(all(test, feature = "test-db"))] mod tests` block in `slack_grant_vault_service.rs`. Match the surrounding tests' setup style (they use `#[sqlx::test]` and a local profile-seeding helper — reuse whatever helper the neighbouring `mint_*` tests use; do **not** invent a new one).

```rust
#[sqlx::test]
async fn take_refresh_token_returns_the_sealed_token_in_plaintext(pool: PgPool) {
    let key = test_key();
    let profile_id = seed_profile(&pool).await;
    store_grant(
        &pool,
        &key,
        NewGrant {
            profile_id,
            slack_principal_id: "slack:T1:U1",
            refresh_token: "rt-plaintext",
            access_token: "at-plaintext",
            access_ttl_secs: Some(3600),
        },
    )
    .await
    .expect("store");

    let mut conn = pool.acquire().await.expect("acquire");
    let got = take_refresh_token_for_disconnect(&mut conn, &key, "slack:T1:U1")
        .await
        .expect("take");
    assert_eq!(got.as_deref(), Some("rt-plaintext"));
}

#[sqlx::test]
async fn take_refresh_token_returns_none_for_an_unknown_principal(pool: PgPool) {
    let key = test_key();
    let mut conn = pool.acquire().await.expect("acquire");
    let got = take_refresh_token_for_disconnect(&mut conn, &key, "slack:T9:U9")
        .await
        .expect("take");
    assert!(got.is_none(), "an unvaulted principal must yield None, not an error");
}
```

Before writing, read the existing tests in this module and reuse their exact helper names for key construction and profile seeding. `test_key()` and `seed_profile()` above are placeholders for whatever they are actually called — grep for them:

```bash
grep -n "fn test_key\|fn seed_profile\|VaultKey::from_base64" crates/temper-services/src/services/slack_grant_vault_service.rs
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db take_refresh_token_returns_the_sealed_token`
Expected: FAIL — `cannot find function 'take_refresh_token_for_disconnect'`

- [ ] **Step 3: Write minimal implementation**

Add to `slack_grant_vault_service.rs`, after `mint_access_token`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Ensure Docker Postgres is up first:

```bash
cargo make docker-up
cargo nextest run -p temper-services --features test-db -E 'test(take_refresh_token_returns_the_sealed_token) + test(take_refresh_token_returns_none_for_an_unknown_principal)'
```
Expected: PASS, 2 tests

- [ ] **Step 5: Regenerate the sqlx cache and commit**

This adds a new `sqlx::query!` macro in a lib target, so the workspace cache must be regenerated.

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
git add crates/temper-services/src/services/slack_grant_vault_service.rs .sqlx
git commit -m "feat(slack): read+decrypt the stored RT for the disconnect path"
```

---

## Task 3: The disconnect service

**Files:**
- Create: `crates/temper-services/src/services/slack_disconnect_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Consumes: `oauth_client::revoke_grant` (Task 1), `slack_grant_vault_service::take_refresh_token_for_disconnect` (Task 2), `slack_link_service::SLACK_AUTH_PROVIDER`, `VaultKey`, `AuthMode`, `ApiResult`.
- Produces:
  - `pub struct DisconnectOutcome { pub was_linked: bool, pub grant_deleted: bool, pub intents_deleted: i64, pub idp_revoked: bool }`
  - `pub struct DisconnectRequest<'a> { pub slack_principal_id: &'a str, pub key: &'a VaultKey, pub mode: AuthMode, pub revoke_url: String, pub client_id: &'a str }`
  - `pub async fn disconnect_slack_principal(pool: &PgPool, req: DisconnectRequest<'_>) -> ApiResult<DisconnectOutcome>`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/src/services/slack_disconnect_service.rs` with only the test module for now, so the test drives the shape:

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    // Reuse the vault module's fixtures rather than inventing parallel ones.
    // Read slack_grant_vault_service.rs's test module and mirror its helpers.

    #[sqlx::test]
    async fn disconnecting_an_unlinked_principal_is_a_quiet_no_op(pool: PgPool) {
        let out = disconnect_slack_principal(
            &pool,
            DisconnectRequest {
                slack_principal_id: "slack:T0:UNEVER",
                key: &test_key(),
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

    #[sqlx::test]
    async fn disconnect_deletes_link_grant_and_intents_together(pool: PgPool) {
        let principal = "slack:T1:U1";
        let key = test_key();
        let profile_id = seed_profile(&pool).await;

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
        assert!(!out.idp_revoked, "the unreachable IdP must report not-revoked");

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
        let alive: bool =
            sqlx::query_scalar("SELECT is_active FROM kb_profiles WHERE id = $1")
                .bind(profile_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(alive, "disconnect is not deactivation");
    }

    #[sqlx::test]
    async fn disconnecting_twice_is_not_an_error(pool: PgPool) {
        let principal = "slack:T2:U2";
        let key = test_key();
        let profile_id = seed_profile(&pool).await;
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

        let first = disconnect_slack_principal(&pool, req()).await.expect("first");
        assert!(first.was_linked);
        let second = disconnect_slack_principal(&pool, req()).await.expect("second");
        assert!(!second.was_linked, "the second disconnect is a quiet no-op");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db -E 'binary(temper-services) and test(disconnect)'`
Expected: FAIL to compile — `disconnect_slack_principal` not found

- [ ] **Step 3: Write minimal implementation**

Prepend to the same file, above the test module:

```rust
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
```

Register the module. In `crates/temper-services/src/services/mod.rs`, add alongside the other slack modules:

```rust
pub mod slack_disconnect_service;
```

Confirm `sha2` is a dependency of temper-services:

```bash
grep -n "^sha2" crates/temper-services/Cargo.toml
```

If absent, add `sha2 = { workspace = true }` under `[dependencies]`.

Note the import style: this crate's services use `use crate::error::{ApiError, ApiResult};` and `use super::<sibling_module>;` (see `slack_grant_vault_service.rs:14-16`). The code above imports only `ApiResult` because the `revoke_grant` failure is matched and logged via `Display`, never constructed — if you find yourself needing `ApiError`, add it to that same `use crate::error::{…}` line rather than a second import.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(disconnecting_an_unlinked_principal_is_a_quiet_no_op) + test(disconnect_deletes_link_grant_and_intents_together) + test(disconnecting_twice_is_not_an_error)'
```
Expected: PASS, 3 tests

- [ ] **Step 5: Regenerate sqlx cache and commit**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
git add crates/temper-services/src/services/slack_disconnect_service.rs crates/temper-services/src/services/mod.rs crates/temper-services/Cargo.toml .sqlx
git commit -m "feat(slack): disconnect service — revoke then delete identity, grant and intents"
```

---

## Task 4: Drop the dead `revoke`

Disconnect deletes the vault row, so the soft-revoke flag has no remaining purpose. It never had a production caller, and `store_grant`'s `ON CONFLICT … SET revoked_at = NULL` (`:137`) already meant the flag did not survive a re-link.

**Files:**
- Modify: `crates/temper-services/src/services/slack_grant_vault_service.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: removal of `pub async fn revoke`.

- [ ] **Step 1: Confirm there are no callers**

```bash
rg -n "slack_grant_vault_service::revoke|vault_service::revoke\b" crates/ tests/ packages/
rg -n "\brevoke\(" crates/temper-services/src/services/slack_grant_vault_service.rs
```
Expected: matches only inside `slack_grant_vault_service.rs` (the fn itself and its two unit tests). If anything else appears, STOP and report — the plan's premise is wrong.

- [ ] **Step 2: Delete the function and its tests**

Remove `pub async fn revoke` (`:285-299`) and the two tests that exercise it: `revoke_reports_the_transition_once` (`:537`) and `revoke_of_an_unknown_principal_is_a_no_op` (`:552`).

Leave the `revoked_at` **column** and `mint_access_token`'s `revoked_at.is_some()` check in place — the column still guards any row that was flagged before this change, and `mint` must keep honouring it.

- [ ] **Step 3: Verify the crate still builds and its tests pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'binary(temper-services)'
```
Expected: PASS, with the two removed tests absent from the count

- [ ] **Step 4: Regenerate sqlx cache**

Deleting the last caller of a query orphans its `.sqlx` entry; the workspace prepare prunes it.

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/services/slack_grant_vault_service.rs .sqlx
git commit -m "refactor(slack): drop the never-wired vault revoke flag in favour of disconnect's delete"
```

---

## Task 5: The intents reaper

The partial index `idx_slack_link_intents_unconsumed` was created for a reaper that was never written. Nothing has ever deleted from `kb_slack_link_intents`, so consumed and expired rows — each holding a Slack principal and a PKCE verifier — accumulate forever.

**Files:**
- Modify: `crates/temper-services/src/services/slack_disconnect_service.rs`

**Interfaces:**
- Consumes: `ApiResult`.
- Produces: `pub async fn reap_expired_intents(pool: &PgPool) -> ApiResult<i64>`

- [ ] **Step 1: Write the failing test**

Add to the test module in `slack_disconnect_service.rs`:

```rust
#[sqlx::test]
async fn the_reaper_sweeps_expired_and_consumed_intents_but_spares_live_ones(pool: PgPool) {
    use crate::services::slack_link_service::create_intent;

    // Live — must survive.
    create_intent(&pool, "slack:T1:ULIVE", "v-live", std::time::Duration::from_secs(900))
        .await
        .expect("live intent");

    // Expired — must be swept.
    create_intent(&pool, "slack:T1:UEXP", "v-exp", std::time::Duration::from_secs(900))
        .await
        .expect("expiring intent");
    sqlx::query("UPDATE kb_slack_link_intents SET expires_at = now() - interval '1 hour' WHERE slack_principal_id = $1")
        .bind("slack:T1:UEXP")
        .execute(&pool)
        .await
        .unwrap();

    // Consumed but not yet expired — must be swept (its purpose is spent).
    create_intent(&pool, "slack:T1:UUSED", "v-used", std::time::Duration::from_secs(900))
        .await
        .expect("consumed intent");
    sqlx::query("UPDATE kb_slack_link_intents SET consumed_at = now() WHERE slack_principal_id = $1")
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db the_reaper_sweeps_expired`
Expected: FAIL — `cannot find function 'reap_expired_intents'`

- [ ] **Step 3: Write minimal implementation**

Add to `slack_disconnect_service.rs`:

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-services --features test-db the_reaper_sweeps_expired`
Expected: PASS

- [ ] **Step 5: Regenerate sqlx cache and commit**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
git add crates/temper-services/src/services/slack_disconnect_service.rs .sqlx
git commit -m "feat(slack): reap expired and consumed link intents"
```

---

## Task 6: The response DTO

**Files:**
- Create: `crates/temper-core/src/types/slack.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

**Interfaces:**
- Produces: `pub struct SlackDisconnectResponse { pub was_linked: bool, pub grant_deleted: bool, pub intents_deleted: i64, pub idp_revoked: bool }`

- [ ] **Step 1: Create the DTO**

Create `crates/temper-core/src/types/slack.rs`:

```rust
//! Wire types for the Slack account-link surface.

use serde::{Deserialize, Serialize};

/// The result of a disconnect, as returned to CLI callers.
///
/// Every field is an observation of what actually happened, so the CLI can tell
/// the user the truth rather than echoing a canned success message. In
/// particular `idp_revoked = false` is a normal, non-error outcome: the local
/// unbind is complete either way.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackDisconnectResponse {
    /// An identity row existed and was removed.
    pub was_linked: bool,
    /// A stored grant existed and was destroyed.
    pub grant_deleted: bool,
    /// How many pending link intents were swept.
    pub intents_deleted: i64,
    /// The IdP acknowledged the revocation. `false` means the grant may remain
    /// live at the IdP until it expires; the local copy is destroyed regardless.
    pub idp_revoked: bool,
}

/// Request body for the admin disconnect endpoint.
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackDisconnectRequest {
    /// The whole opaque Slack principal (`slack:<team>:<user>`, 2–4 segments).
    /// Never split this value.
    pub slack_principal_id: String,
}
```

Deliberately **no** `ts-rs` and **no** `schemars` derives: no UI or MCP surface consumes these, and adding `ts-rs` would create a fourth generated artifact (`cargo make generate-ts-types`) that `cargo make check` does not gate.

Register in `crates/temper-core/src/types/mod.rs`, matching the surrounding style:

```rust
pub mod slack;
```

Then check whether that module file re-exports its types (most do) and follow suit:

```bash
grep -n "pub use team::" crates/temper-core/src/types/mod.rs
```

If sibling modules are re-exported, add `pub use slack::{SlackDisconnectRequest, SlackDisconnectResponse};`.

- [ ] **Step 2: Verify it compiles under every feature combination**

```bash
cargo check -p temper-core --all-features
cargo check -p temper-core --no-default-features
```
Expected: both succeed

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/types/slack.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(slack): add disconnect request/response DTOs"
```

---

## Task 7: The two HTTP handlers

**Files:**
- Create: `crates/temper-api/src/handlers/slack_disconnect.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`, `crates/temper-api/src/routes.rs`

**Interfaces:**
- Consumes: `slack_disconnect_service::{disconnect_slack_principal, DisconnectRequest}`, `SlackDisconnectResponse`, `SlackDisconnectRequest`, `AuthUser`, `access_service::is_system_admin`.
- Produces: `pub async fn disconnect_me`, `pub async fn admin_disconnect`.

- [ ] **Step 1: Write the handlers**

Create `crates/temper-api/src/handlers/slack_disconnect.rs`:

```rust
//! Disconnect surfaces — self-serve and admin.
//!
//! Both dispatch to the one service chokepoint. The self-serve arm never
//! accepts a principal from the caller: it derives it from the caller's own
//! auth-link row, so naming someone else's principal is not expressible.

use axum::extract::State;
use axum::Json;
use temper_core::types::ids::ProfileId;
use temper_core::types::slack::{SlackDisconnectRequest, SlackDisconnectResponse};
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::access_service;
use temper_services::services::slack_disconnect_service::{
    disconnect_slack_principal, DisconnectOutcome, DisconnectRequest,
};
use temper_services::services::slack_link_service::SLACK_AUTH_PROVIDER;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

fn to_response(outcome: DisconnectOutcome) -> SlackDisconnectResponse {
    SlackDisconnectResponse {
        was_linked: outcome.was_linked,
        grant_deleted: outcome.grant_deleted,
        intents_deleted: outcome.intents_deleted,
        idp_revoked: outcome.idp_revoked,
    }
}

/// Build the revocation URL for the active provider mode.
///
/// `LinkProvider` carries no mode field, so mode comes from `AuthConfig`
/// directly. In `TemperAs` mode this URL is never dialled — the service revokes
/// locally — but we still produce a well-formed value rather than an `Option`
/// the caller would have to unwrap.
fn revoke_url(state: &AppState) -> String {
    let base = state.config.auth.issuer.trim_end_matches('/');
    format!("{base}/oauth/revoke")
}

/// Disconnect the caller's own Slack link.
#[utoipa::path(
    delete,
    path = "/api/auth/slack/link/me",
    tag = "auth",
    responses(
        (status = 200, description = "Disconnect completed (idempotent)", body = SlackDisconnectResponse),
        (status = 401, description = "Authentication required"),
        (status = 503, description = "Slack account linking is not configured"),
    )
)]
pub async fn disconnect_me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<SlackDisconnectResponse>> {
    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    let profile_id = ProfileId::from(auth.0.profile.id);

    // Derive the principal from the caller's OWN link row. This is the whole
    // authorization story for the self-serve arm: there is no input to forge.
    let principal: Option<String> = sqlx::query_scalar!(
        r#"
        SELECT auth_provider_user_id
          FROM kb_profile_auth_links
         WHERE profile_id = $1
           AND auth_provider = $2
        "#,
        auth.0.profile.id,
        SLACK_AUTH_PROVIDER
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(principal) = principal else {
        // Idempotent: nothing linked, nothing to do.
        return Ok(Json(SlackDisconnectResponse {
            was_linked: false,
            grant_deleted: false,
            intents_deleted: 0,
            idp_revoked: false,
        }));
    };

    tracing::info!(%profile_id, "self-serve slack disconnect requested");

    let outcome = disconnect_slack_principal(
        &state.pool,
        DisconnectRequest {
            slack_principal_id: &principal,
            key: &cfg.vault_key,
            mode: state.config.auth.mode,
            revoke_url: revoke_url(&state),
            client_id: &cfg.client_id,
        },
    )
    .await?;

    Ok(Json(to_response(outcome)))
}

/// Disconnect any principal. Operator path — offboarding and stuck users.
#[utoipa::path(
    post,
    path = "/api/admin/slack/links/disconnect",
    tag = "admin",
    request_body = SlackDisconnectRequest,
    responses(
        (status = 200, description = "Disconnect completed (idempotent)", body = SlackDisconnectResponse),
        (status = 403, description = "Caller is not a system admin"),
    )
)]
pub async fn admin_disconnect(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SlackDisconnectRequest>,
) -> ApiResult<Json<SlackDisconnectResponse>> {
    // Auth before any mutation. Load-bearing: the gated router admits everyone
    // under access_mode='open' (routes.rs:168-169), so this is the real gate.
    if !access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await? {
        return Err(ApiError::Forbidden);
    }

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    crate::handlers::slack_link::validate_slack_principal(&body.slack_principal_id)?;

    tracing::info!(
        principal = %body.slack_principal_id,
        actor = %auth.0.profile.id,
        "admin slack disconnect requested"
    );

    let outcome = disconnect_slack_principal(
        &state.pool,
        DisconnectRequest {
            slack_principal_id: &body.slack_principal_id,
            key: &cfg.vault_key,
            mode: state.config.auth.mode,
            revoke_url: revoke_url(&state),
            client_id: &cfg.client_id,
        },
    )
    .await?;

    Ok(Json(to_response(outcome)))
}
```

`validate_slack_principal` is currently private in `slack_link.rs:133`. Make it `pub(crate)`:

```rust
pub(crate) fn validate_slack_principal(principal: &str) -> Result<(), ApiError> {
```

Register the handler module in `crates/temper-api/src/handlers/mod.rs`:

```rust
pub mod slack_disconnect;
```

- [ ] **Step 2: Register the routes**

In `crates/temper-api/src/routes.rs`, add to `auth_only_routes()` (`:25`):

```rust
        .routes(routes!(handlers::slack_disconnect::disconnect_me))
```

and to `gated_routes()` (`:44`):

```rust
        .routes(routes!(handlers::slack_disconnect::admin_disconnect))
```

Both use `.routes(routes!(…))`, not plain `.route(…)` — that keeps them inside the OpenAPI contract and out of the `check-openapi-routes.sh` allowlist. Do **not** edit that allowlist.

- [ ] **Step 3: Verify it compiles and the route gate passes**

```bash
cargo check -p temper-api --all-features
bash .github/scripts/check-openapi-routes.sh
```
Expected: both succeed. If the route check fails naming your paths, you used `.route(` instead of `.routes(`.

- [ ] **Step 4: Regenerate the OpenAPI artifacts**

Two new documented endpoints restale three committed artifacts.

```bash
cargo make openapi
```

If Docker is unavailable the gem step prints a NOTE and skips; run `cargo make openapi-rb` later on a machine with Docker before opening the PR, or the `test-ruby` CI job will fail.

- [ ] **Step 5: Stage the regenerated artifacts and commit**

The drift gates compare against **git**, not against a fresh build — a correctly regenerated artifact still fails `cargo make check` while unstaged.

```bash
git add crates/temper-api/src/handlers/slack_disconnect.rs \
        crates/temper-api/src/handlers/mod.rs \
        crates/temper-api/src/handlers/slack_link.rs \
        crates/temper-api/src/routes.rs \
        openapi.json clients/temper-rb clients/temper-ts
git commit -m "feat(slack): self-serve and admin disconnect endpoints"
```

- [ ] **Step 6: Regenerate the per-crate sqlx cache**

`disconnect_me` adds a `query_scalar!` in temper-api.

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-api
git add .sqlx crates/temper-api/.sqlx
git commit -m "chore: refresh sqlx caches for the disconnect endpoints"
```

Note `cargo make prepare-api` materialises many untracked entries under `crates/temper-api/.sqlx`; stage that directory explicitly as above rather than running a bare `git add .sqlx`.

---

## Task 8: The reaper cron endpoint

**Files:**
- Create: handler in `crates/temper-api/src/handlers/slack_disconnect.rs` (append)
- Modify: `crates/temper-api/src/handlers/embed.rs`, `crates/temper-api/src/routes.rs`, `vercel.json`

**Interfaces:**
- Consumes: `reap_expired_intents` (Task 5), `require_dispatch_secret`.
- Produces: `pub async fn reap_intents` on `GET|POST /api/slack/intents/reap`.

- [ ] **Step 1: Widen `require_dispatch_secret`**

In `crates/temper-api/src/handlers/embed.rs:58`, change the visibility. It is the shared Vercel cron-secret gate, not an embed-specific one:

```rust
/// Gate a cron/ops endpoint on the shared Vercel cron bearer secret.
///
/// Fail-closed: an unset secret disables the endpoint rather than opening it.
pub(crate) fn require_dispatch_secret(
    state: &AppState,
    headers: &HeaderMap,
    label: &str,
) -> ApiResult<()> {
```

Reuse the existing `EMBED_DISPATCH_SECRET` rather than introducing a new env var — a new fail-closed variable would become a deploy-time prerequisite, exactly the hazard that took the T3 deploy dark.

- [ ] **Step 2: Write the handler**

Append to `crates/temper-api/src/handlers/slack_disconnect.rs`:

```rust
/// Response for the intents reaper cron.
#[derive(Debug, serde::Serialize)]
pub struct ReapSummary {
    pub swept: i64,
}

/// Cron: sweep expired and consumed Slack link intents.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router,
/// matching the embed crons. Vercel Cron invokes with GET; POST exists for
/// manual ops.
pub async fn reap_intents(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Json<ReapSummary>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "slack intents reap")?;
    let swept =
        temper_services::services::slack_disconnect_service::reap_expired_intents(&state.pool)
            .await?;
    Ok(Json(ReapSummary { swept }))
}
```

- [ ] **Step 3: Register the route and the allowlist entry**

In `crates/temper-api/src/routes.rs`, extend `embed_internal_routes()` (`:289`) — it is the bare `Router<AppState>` merged into both `create_app` and `create_internal_app`:

```rust
        .route(
            "/api/slack/intents/reap",
            get(handlers::slack_disconnect::reap_intents)
                .post(handlers::slack_disconnect::reap_intents),
        )
```

This one **is** a plain `.route`, so its path must go on the allowlist. Add to the `ALLOWLIST` heredoc in `.github/scripts/check-openapi-routes.sh` (after `/api/embed/warm`):

```
/api/slack/intents/reap
```

- [ ] **Step 4: Wire the cron in `vercel.json`**

Add to `crons` — hourly is ample for a retention sweep:

```json
{ "path": "/api/slack/intents/reap", "schedule": "0 * * * *" }
```

And to `routes`, **before** the catch-all, so it lands on the 300s `api/internal.rs` function rather than the 60s public one:

```json
{ "src": "/api/slack/intents/reap", "dest": "/api/internal" }
```

Both entries are required: the `{ "handle": "filesystem" }` entry means anything not explicitly routed falls through to the `/(.*)` → `/api/axum` catch-all.

- [ ] **Step 5: Verify**

```bash
cargo check -p temper-api --all-features
bash .github/scripts/check-openapi-routes.sh
python3 -c "import json;json.load(open('vercel.json'));print('vercel.json parses')"
```
Expected: all three succeed

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/handlers/slack_disconnect.rs \
        crates/temper-api/src/handlers/embed.rs \
        crates/temper-api/src/routes.rs \
        .github/scripts/check-openapi-routes.sh \
        vercel.json
git commit -m "feat(slack): hourly cron reaping expired and consumed link intents"
```

---

## Task 9: The `temper-client` sub-client

**Files:**
- Create: `crates/temper-client/src/slack.rs`
- Modify: `crates/temper-client/src/lib.rs`

**Interfaces:**
- Consumes: `HttpClient` (`resolve_token`, `delete`, `post`, `send_json`).
- Produces: `SlackClient<'a>` with `disconnect_me()` and `admin_disconnect(principal)`, plus `TemperClient::slack()`.

- [ ] **Step 1: Write the sub-client**

Create `crates/temper-client/src/slack.rs`, mirroring `machine.rs`'s structure exactly:

```rust
//! Slack account-link client surface.

use reqwest::Method;
use temper_core::types::slack::{SlackDisconnectRequest, SlackDisconnectResponse};

use crate::error::Result;
use crate::http::HttpClient;

pub struct SlackClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SlackClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlackClient").finish_non_exhaustive()
    }
}

impl<'a> SlackClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Disconnect the caller's own Slack link. Idempotent.
    pub async fn disconnect_me(&self) -> Result<SlackDisconnectResponse> {
        let token = self.http.resolve_token()?;
        let path = "/api/auth/slack/link/me";
        let req = self.http.delete(path);
        self.http
            .send_json(&Method::DELETE, path, req, Some(&token))
            .await
    }

    /// Disconnect any principal. Requires system admin. Idempotent.
    pub async fn admin_disconnect(
        &self,
        slack_principal_id: &str,
    ) -> Result<SlackDisconnectResponse> {
        let token = self.http.resolve_token()?;
        let path = "/api/admin/slack/links/disconnect";
        let body = SlackDisconnectRequest {
            slack_principal_id: slack_principal_id.to_string(),
        };
        let req = self.http.post(path).json(&body);
        self.http
            .send_json(&Method::POST, path, req, Some(&token))
            .await
    }
}
```

Both endpoints return a JSON body, so `send_json` is correct — it would fail to decode a 204, which is why neither handler returns one.

- [ ] **Step 2: Register the sub-client**

In `crates/temper-client/src/lib.rs`, add the module next to the others:

```rust
pub mod slack;
```

and the accessor on `TemperClient`, matching `machine_clients()` (`lib.rs:169-171`):

```rust
    pub fn slack(&self) -> slack::SlackClient<'_> {
        slack::SlackClient::new(&self.http)
    }
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p temper-client --all-features
```
Expected: success

- [ ] **Step 4: Commit**

```bash
git add crates/temper-client/src/slack.rs crates/temper-client/src/lib.rs
git commit -m "feat(slack): temper-client disconnect methods"
```

---

## Task 10: The two CLI commands

**Files:**
- Create: `crates/temper-cli/src/commands/slack.rs`, `crates/temper-cli/src/commands/admin_slack.rs`
- Modify: `crates/temper-cli/src/cli.rs`, `crates/temper-cli/src/main.rs`, `crates/temper-cli/src/commands/mod.rs`

**Interfaces:**
- Consumes: `TemperClient::slack()`, `OutputFormat`, `crate::format::render`, `crate::output::warning`, `crate::commands::client_err`, `actions::runtime::with_client`.
- Produces: `Commands::Slack { action: SlackAction }`, `AdminAction::Slack { action: AdminSlackAction }`.

- [ ] **Step 1: Add the clap definitions**

In `crates/temper-cli/src/cli.rs`, add a top-level variant to `Commands` (near `Auth` at `:236`):

```rust
    /// Manage the Slack account link
    Slack {
        #[command(subcommand)]
        action: SlackAction,
    },
```

and the enum, alongside the other action enums:

```rust
#[derive(Debug, clap::Subcommand)]
pub enum SlackAction {
    /// Disconnect your Slack account from this temper profile.
    Disconnect,
}
```

Add to `AdminAction` (`:1007`), matching the `Machine` variant's shape exactly:

```rust
    /// Administer Slack account links
    Slack {
        #[command(subcommand)]
        action: AdminSlackAction,
    },
```

and:

```rust
#[derive(Debug, clap::Subcommand)]
pub enum AdminSlackAction {
    /// Disconnect a Slack principal from its temper profile. Idempotent.
    Disconnect { principal: String },
}
```

`Disconnect { principal: String }` takes a bare positional with no clap attributes, matching `AdminMachineAction::Revoke { id: String }`.

- [ ] **Step 2: Write the self-serve command**

Create `crates/temper-cli/src/commands/slack.rs`:

```rust
use crate::error::Result;
use crate::format::OutputFormat;

/// Disconnect the caller's own Slack link.
///
/// Caveats go to stderr via `output::warning`, never to stdout — temper
/// defaults to JSON on a non-TTY stdout, and a hint on stdout corrupts it.
pub async fn disconnect_remote(
    client: &temper_client::TemperClient,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .slack()
        .disconnect_me()
        .await
        .map_err(crate::commands::client_err)?;

    let was_linked = row.was_linked;
    let idp_revoked = row.idp_revoked;
    println!("{}", crate::format::render(&row, fmt)?);

    if !was_linked {
        crate::output::warning("No Slack link was found for your profile — nothing to disconnect.");
        return Ok(());
    }
    if !idp_revoked {
        crate::output::warning(
            "The identity provider did not confirm revocation. Your stored grant was destroyed \
             regardless, so temper can no longer use it.",
        );
    }
    crate::output::warning(
        "Disconnect stops future access-token mints. An access token already issued remains \
         valid until it expires (up to one hour) — this is not an instant cutoff.",
    );
    Ok(())
}
```

- [ ] **Step 3: Write the admin command**

Create `crates/temper-cli/src/commands/admin_slack.rs`:

```rust
use crate::error::Result;
use crate::format::OutputFormat;

/// Disconnect any Slack principal. Requires system admin.
///
/// The principal is opaque and has 2–4 segments — it is passed whole and never
/// split.
pub async fn disconnect_remote(
    client: &temper_client::TemperClient,
    principal: &str,
    fmt: OutputFormat,
) -> Result<()> {
    let row = client
        .slack()
        .admin_disconnect(principal)
        .await
        .map_err(crate::commands::client_err)?;

    let was_linked = row.was_linked;
    let idp_revoked = row.idp_revoked;
    println!("{}", crate::format::render(&row, fmt)?);

    if !was_linked {
        crate::output::warning(
            "No link existed for that principal — the disconnect was a no-op (this is not an error).",
        );
    }
    if !idp_revoked {
        crate::output::warning(
            "The identity provider did not confirm revocation; revoke out-of-band if that matters. \
             The local grant was destroyed regardless.",
        );
    }
    crate::output::warning(
        "The profile, its teams and its resources are untouched — disconnect unbinds an identity, \
         it does not deactivate an account.",
    );
    Ok(())
}
```

Register both in `crates/temper-cli/src/commands/mod.rs`:

```rust
pub mod admin_slack;
pub mod slack;
```

- [ ] **Step 4: Wire the dispatch arms**

In `crates/temper-cli/src/main.rs`, add alongside the other `Commands` arms:

```rust
            Commands::Slack { action } => match action {
                SlackAction::Disconnect => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::slack::disconnect_remote(client, output_format)
                                .await
                        })
                    })
                }
            },
```

and inside the `AdminAction` match:

```rust
                AdminAction::Slack { action } => match action {
                    AdminSlackAction::Disconnect { principal } => {
                        temper_cli::actions::runtime::with_client(|client| {
                            Box::pin(async move {
                                temper_cli::commands::admin_slack::disconnect_remote(
                                    client,
                                    &principal,
                                    output_format,
                                )
                                .await
                            })
                        })
                    }
                },
```

Import `SlackAction` and `AdminSlackAction` wherever the sibling action enums are imported in `main.rs`.

- [ ] **Step 5: Verify the CLI builds and the help renders**

```bash
cargo build -p temper-cli --all-features
./target/debug/temper slack disconnect --help
./target/debug/temper admin slack disconnect --help
```
Expected: both print help without error

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs \
        crates/temper-cli/src/commands/mod.rs \
        crates/temper-cli/src/commands/slack.rs \
        crates/temper-cli/src/commands/admin_slack.rs
git commit -m "feat(slack): temper slack disconnect and temper admin slack disconnect"
```

---

## Task 11: Stop `output::hint` corrupting JSON stdout

`output::hint` writes to **stdout**, so any command that renders JSON and then emits a hint produces output that is not valid JSON. Temper defaults to JSON on a non-TTY stdout — which is how agents invoke it — so this is a live correctness bug, not a style nit.

**Files:**
- Modify: `crates/temper-cli/src/output/mod.rs`

**Interfaces:**
- Produces: `hint` writes to stderr.

- [ ] **Step 1: Find every caller**

```bash
rg -n "output::hint\(|crate::output::hint\(" crates/temper-cli/src/ | tee /tmp/hint-callers.txt
wc -l /tmp/hint-callers.txt
```
Record the count — you will re-check it after the change.

- [ ] **Step 2: Write the failing test**

Add to `crates/temper-cli/src/output/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    /// `hint` must not write to stdout: temper defaults to JSON on a non-TTY
    /// stdout, and a hint interleaved there makes the payload unparseable.
    /// Verified by reading the source of `hint` rather than capturing the
    /// process's real stdout, which the test harness owns.
    #[test]
    fn hint_writes_to_stderr() {
        let src = include_str!("mod.rs");
        let hint_fn = src
            .split("pub fn hint(")
            .nth(1)
            .expect("hint must exist");
        let body = &hint_fn[..hint_fn.find('}').expect("hint body")];
        assert!(
            body.contains("stderr"),
            "hint must write to stderr, got: {body}"
        );
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli hint_writes_to_stderr`
Expected: FAIL — the body contains `stdout`

- [ ] **Step 4: Change `hint` to stderr**

In `crates/temper-cli/src/output/mod.rs:51-55`:

```rust
/// Print a hint/suggestion (dimmed, for guidance text).
///
/// Goes to **stderr**, not stdout: temper defaults to JSON output on a non-TTY
/// stdout (how agents invoke it), and a hint written there corrupts the payload.
/// Guidance is for humans; the payload is for parsers.
pub fn hint(msg: impl std::fmt::Display) {
    let mut out = anstream::stderr().lock();
    writeln!(out, "{HINT}{msg}{HINT:#}").ok();
}
```

- [ ] **Step 5: Run the test and the full CLI suite**

```bash
cargo nextest run -p temper-cli hint_writes_to_stderr
cargo nextest run -p temper-cli
```
Expected: PASS. If any e2e test asserted a hint on stdout it will now fail — fix the assertion to read stderr, and note it in the commit body.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/output/mod.rs
git commit -m "fix(cli): route output::hint to stderr so it cannot corrupt JSON stdout"
```

---

## Task 12: E2E harness extension and disconnect tests

`slack_link_test.rs` uses `SlackLinkApp`, which is **not** `common::E2eTestApp` — it has no `token`, `config`, or `vault_dir`, so `run_temper_cli` cannot be called from it. And there is no token-parameterised variant anywhere.

**Files:**
- Modify: `tests/e2e/tests/common/mod.rs`, `tests/e2e/tests/slack_link_test.rs`

**Interfaces:**
- Consumes: `run_temper_cli`'s internals.
- Produces: `pub async fn run_temper_cli_with_token(api_url: &str, token: &str, config_dir: &Path, args: &[&str]) -> std::io::Result<std::process::Output>`

- [ ] **Step 1: Extract the token-parameterised runner**

In `tests/e2e/tests/common/mod.rs`, refactor `run_temper_cli` (`:92-117`) so the existing signature delegates to a new one. Read the current body first and preserve `temper_bin_path()` and the `TEMPER_GLOBAL_CONFIG` materialisation exactly.

```rust
/// Run the real `temper` binary against an arbitrary API URL and token.
///
/// `run_temper_cli` is the convenience wrapper for `E2eTestApp`; this is the
/// form harnesses with their own app struct (e.g. `SlackLinkApp`) can use.
pub async fn run_temper_cli_with_token(
    api_url: &str,
    token: &str,
    config_path: &std::path::Path,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    let bin = temper_bin_path();
    let url = api_url.to_string();
    let token = token.to_string();
    let config_path = config_path.to_path_buf();
    let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    tokio::task::spawn_blocking(move || {
        std::process::Command::new(&bin)
            .env("TEMPER_API_URL", &url)
            .env("TEMPER_TOKEN", &token)
            .env("TEMPER_GLOBAL_CONFIG", &config_path)
            .args(&args_owned)
            .output()
    })
    .await
    .expect("spawn_blocking")
}
```

Then rewrite `run_temper_cli` to build its config file as it does today and delegate:

```rust
pub async fn run_temper_cli(
    app: &E2eTestApp,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    let config_path = app.materialize_cli_config();  // keep the existing inline logic
    run_temper_cli_with_token(
        &format!("http://{}", app.addr),
        &app.token,
        &config_path,
        args,
    )
    .await
}
```

Keep the existing config-materialisation logic verbatim; only move it. If it is inline rather than a method, leave it inline in `run_temper_cli`.

- [ ] **Step 2: Give `SlackLinkApp` what the runner needs**

In `tests/e2e/tests/slack_link_test.rs`, add a `TempDir` field to `SlackLinkApp` (`:73-91`) and a helper that writes a minimal global config, mirroring what `run_temper_cli` writes for `E2eTestApp`. Read that logic and copy its shape.

Also change `provision_profile` (`:268`) to return the token it mints, so tests can authenticate as that user:

```rust
async fn provision_profile(app: &SlackLinkApp, sub: &str, email: &str) -> String {
    let token = sign_idp_access_token(&app.issuer(), sub, email);
    let res = app
        .http
        .get(format!("http://{}/api/profile", app.addr))
        .bearer_auth(&token)
        .send()
        .await
        .expect("GET /api/profile");
    assert_eq!(
        res.status(),
        200,
        "the token user must auto-provision on first authenticated request"
    );
    token
}
```

Update its existing call sites — they currently discard the return, which still compiles with a `let _ =` or by ignoring the value; prefer binding it where useful.

- [ ] **Step 3: Write the failing e2e tests**

Add to `tests/e2e/tests/slack_link_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn disconnect_unbinds_the_principal_and_the_next_mention_prompts_to_link(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-disconnect";
    let email = "disconnect-1a2b3c@example.invalid";
    let token = provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    // Link.
    let body = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback")
        .text()
        .await
        .expect("callback body");
    assert!(body.contains("Linked as"), "the link must succeed: {body}");
    assert_eq!(count_slack_links(&pool).await, 1);

    // Disconnect via the real CLI binary.
    let out = common::run_temper_cli_with_token(
        &format!("http://{}", app.addr),
        &token,
        &app.cli_config_path(),
        &["slack", "disconnect", "--format", "json"],
    )
    .await
    .expect("cli disconnect");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["was_linked"], true);

    // Assert absence of the rows, not the success message.
    assert_eq!(count_slack_links(&pool).await, 0, "identity row must be gone");
    let grants: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_slack_grant_vault WHERE slack_principal_id = $1")
            .bind(SLACK_PRINCIPAL)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(grants, 0, "the sealed grant must be destroyed");
    assert_eq!(count_intents(&pool).await, 0, "intents must be swept");

    // The next mention is offered a fresh link — the normal T2 flow, no special path.
    let res = post_link_state(&app, SLACK_PRINCIPAL, None).await;
    assert_eq!(res.status(), 200);
    let state: serde_json::Value = res.json().await.expect("json");
    assert_eq!(
        state["status"], "unlinked",
        "a disconnected principal must be offered a link again"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn disconnect_leaves_the_profile_and_is_idempotent(pool: PgPool) {
    let app = setup_slack_app(&pool).await;
    let sub = "idp-sub-idempotent";
    let email = "idempotent-9f8e7d@example.invalid";
    let token = provision_profile(&app, sub, email).await;
    stub_token_endpoint(&app, sign_idp_access_token(&app.issuer(), sub, email)).await;

    let _ = app
        .http
        .get(app.callback_url(&mint_state_nonce(&app).await))
        .send()
        .await
        .expect("callback");

    let profiles_before = count_profiles(&pool).await;

    for attempt in 0..2 {
        let out = common::run_temper_cli_with_token(
            &format!("http://{}", app.addr),
            &token,
            &app.cli_config_path(),
            &["slack", "disconnect", "--format", "json"],
        )
        .await
        .expect("cli disconnect");
        assert!(
            out.status.success(),
            "attempt {attempt} must succeed; stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    assert_eq!(
        count_profiles(&pool).await,
        profiles_before,
        "disconnect is not deactivation — the profile must survive"
    );
}
```

- [ ] **Step 4: Run the tests**

The e2e suite spawns the real `temper` binary, which nextest does **not** rebuild. Build it first.

```bash
cargo build -p temper-cli --all-features
cargo make test-e2e-embed
```

Or, scoped:

```bash
cargo build -p temper-cli --all-features
cargo nextest run -p temper-e2e --features test-db,test-embed -E 'binary(slack_link_test)'
```
Expected: PASS, including the pre-existing slack tests

- [ ] **Step 5: Regenerate the e2e sqlx cache and commit**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make prepare-e2e
git add tests/e2e/tests/common/mod.rs tests/e2e/tests/slack_link_test.rs tests/e2e/.sqlx
git commit -m "test(slack): e2e disconnect coverage via the real CLI binary"
```

---

## Task 13: Operator documentation

**Files:**
- Modify: `docs/guides/slack-setup.md`

- [ ] **Step 1: Add a disconnect section**

Append to `docs/guides/slack-setup.md`, matching the surrounding heading level and tone:

```markdown
## Disconnecting an account

A user unbinds their own Slack link:

```bash
temper slack disconnect
```

An operator unbinds any principal — for offboarding, or for a user who linked
the wrong profile:

```bash
temper admin slack disconnect 'slack:T0BHAHEN79C:U0BH6A3L6JF'
```

The principal is opaque and has two to four segments. Pass it whole, quoted.

Both are **idempotent**: disconnecting an already-disconnected principal
succeeds quietly.

### What disconnect does

- Deletes the identity row (`kb_profile_auth_links`).
- Destroys the encrypted grant (`kb_slack_grant_vault`) — the row is deleted,
  not flagged, so the sealed refresh token no longer exists.
- Sweeps any pending link intents for that principal. This is a security step,
  not hygiene: an intent minted before the disconnect would otherwise remain a
  live first-link URL for a now-unlinked principal.
- Attempts to revoke the grant at the identity provider.

### What disconnect does NOT do

- **It is not deactivation.** The profile, its team memberships, and its
  resources are untouched.
- **It is not an instant cutoff.** Revocation stops *future* access-token mints.
  An access token already issued stays valid until its own expiry — up to one
  hour — because JWKS validation consults no revocation list.
- **It does not uninstall the Slack app.** That is a workspace-level admin
  action.

### If the identity provider revocation fails

The response reports `idp_revoked: false` and the CLI warns on stderr. **The
disconnect still succeeded** — the local grant is destroyed either way, so
temper can no longer use it. The grant may remain live at the IdP until it
expires; revoke it from the Auth0 dashboard if that matters for your threat
model. temper deliberately does not retain the token to retry later.

On self-hosted installs (temper-AS mode) revocation is local and atomic, so this
case does not arise.

### Reconnecting

Just mention `@temper` again. The principal is unlinked, so the normal link flow
offers a fresh authorize URL — there is no special reconnect path.

### Intent retention

Expired and consumed link intents are swept hourly by the
`/api/slack/intents/reap` cron, gated on the same `EMBED_DISPATCH_SECRET` bearer
as the embed crons.
```

- [ ] **Step 2: Verify markdown lints**

```bash
cargo make check 2>&1 | tail -30
```

- [ ] **Step 3: Commit**

```bash
git add docs/guides/slack-setup.md
git commit -m "docs(slack): document disconnect, its honest limits, and the intents reaper"
```

---

## Task 14: Full verification

- [ ] **Step 1: Run the complete check suite**

```bash
cargo make check > /tmp/check.log 2>&1; tail -40 /tmp/check.log
```
Expected: green. If a drift gate reports a generated artifact out of date that you already regenerated, it is unstaged — `git add` it and re-run.

- [ ] **Step 2: Run every test tier**

```bash
cargo make test
cargo make test-db
cargo build -p temper-cli --all-features
cargo make test-e2e-embed
```

Note `test-all` shows one pre-existing streaming-embed timeout on this repo; that is not caused by this branch.

- [ ] **Step 3: Check the migration diff**

```bash
git diff --stat origin/main...HEAD -- migrations/
```

Expected: empty, because this design is DML-only. Output here is not automatically wrong — but it means a task added schema that the plan did not anticipate, so stop and verify the sequencing rules in Global Constraints were followed (highest on main, sibling branches, prod's `_sqlx_migrations`, leave a gap, additive-only) before pushing.

- [ ] **Step 4: Reinstall the CLI locally**

```bash
cargo install --path crates/temper-cli --force
temper slack disconnect --help
```

- [ ] **Step 5: Merge main and push**

```bash
git fetch origin
git merge origin/main
cargo make check > /tmp/check-postmerge.log 2>&1; tail -20 /tmp/check-postmerge.log
git push -u origin jct/slack-disconnect
```

Then open a PR. Do not merge locally.

---

## Self-Review Notes

**Spec coverage** — every acceptance criterion from task `019f703c` maps to a task:

| Acceptance criterion | Covered by |
|---|---|
| Link row gone; next mention gets the normal prompt | Task 3, Task 12 (asserts `status: "unlinked"`) |
| Vault row gone, asserted by row absence | Task 3, Task 12 |
| Grant revoked at the IdP | Task 1, Task 3 (best-effort by decision) |
| A principal cannot disconnect another's link | Task 7 (self-serve derives the principal; admin is gated) |
| Disconnecting twice is not an error | Task 3, Task 12 |
| Profile, teams, resources untouched | Task 3, Task 12 |
| Key-management / honest semantics documented | Task 13 |
| Intents cleanup (added by security review) | Task 3, Task 5 |

**Out of scope, deliberately:** `@temper disconnect` in Slack (deferred until T5's HITL confirm machinery exists); the ledger event (task `019f75ec-f82f-73f1-b038-81993e822f5a`, lands after admin-event-sink Task 5); profile deletion; Slack app uninstall.
