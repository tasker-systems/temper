# T2 — Slack↔temper Account Link Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Slack user proves their temper identity once in a browser; temper writes the directory row `slack:<team>:<user> → profile`.

**Architecture:** Two new Rust endpoints in temper-api. `POST /internal/slack/link-intents` (HMAC-gated, agent→API) mints a PKCE pair + opaque state and returns the IdP authorize URL. `GET /api/auth/slack/callback` (browser-facing) consumes the intent, exchanges the code, resolves the profile **lookup-only**, and upserts the auth link. A new `temper-auth` crate holds the PKCE mechanics both temper-client and temper-services need.

**Tech Stack:** Rust (axum, sqlx, reqwest), PostgreSQL, TypeScript (eve@0.18.1 agent).

**Spec:** [`docs/superpowers/specs/2026-07-16-slack-account-link-design.md`](../specs/2026-07-16-slack-account-link-design.md). Read it first — it carries the *why* for every decision below.

## Global Constraints

- **`--all-features` on every build and clippy.** `cargo make check` is the honest local gate.
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.** All public types derive `Debug`.
- **Never `serde_json::json!()` for known-shape data** — define a struct.
- **>5 domain params ⇒ params struct.** `#[expect(clippy::too_many_arguments)]` is a smell to fix.
- **SQL lives in the service layer.** Never inline `sqlx::query!()` in a handler or middleware.
- **Auth before writes.** Authorization precedes any mutation.
- **This repo pins dependency versions per-crate.** There is no `[workspace.dependencies]`; copy version strings from a sibling crate's `Cargo.toml`.
- **`members = ["crates/*", "tests/e2e"]`** — a new directory under `crates/` is picked up with no root edit.
- **NEVER parse `principalId` with `split(":")`.** It has 2–4 segments. Store and compare it whole.
- **sqlx offline cache**: new `sqlx::query!()` macros fail the pre-commit hook with E0282 until the cache is regenerated. Commit intermediate tasks with `--no-verify`, then run the regeneration ritual in Task 10 **in this order**: `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `cargo make prepare-api` → `cargo make prepare-e2e`.
- **`DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`** for `#[sqlx::test]` under bare `cargo` (`cargo make` tasks set it).

---

## File Structure

**PR 1 — `temper-auth` extraction (one atomic commit)**

| File | Responsibility |
|---|---|
| `crates/temper-auth/Cargo.toml` | new crate manifest |
| `crates/temper-auth/src/lib.rs` | module wiring + `AuthError` |
| `crates/temper-auth/src/pkce.rs` | `generate_pkce_pair` |
| `crates/temper-auth/src/authorize.rs` | `AuthorizeParams` + `build_authorize_url` |
| `crates/temper-auth/src/token.rs` | `TokenResponse` wire type |
| `crates/temper-client/src/login.rs` | repointed to temper-auth; loopback flow unchanged |
| `crates/temper-client/Cargo.toml` | + temper-auth; − rand/sha2/base64 if now unused |

**PR 2 — the link flow**

| File | Responsibility |
|---|---|
| `migrations/20260716000040_slack_link_intents.sql` | `kb_slack_link_intents` |
| `crates/temper-services/src/services/slack_link_service.rs` | intent create/consume + link upsert (all SQL) |
| `crates/temper-services/src/oauth_client.rs` | token exchange (the only HTTP in temper-services) |
| `crates/temper-services/src/link_provider.rs` | mode-aware authorize/token URL derivation |
| `crates/temper-services/src/auth/mod.rs` | `authenticate_token_existing_only` (seam preserved) |
| `crates/temper-services/src/services/profile_service.rs` | `resolve_existing_human_from_claims` |
| `crates/temper-services/src/config.rs` | 3 new env fields |
| `crates/temper-api/src/middleware/internal_auth.rs` | extract shared verify; add slack-link gate |
| `crates/temper-api/src/handlers/slack_link.rs` | both handlers |
| `crates/temper-api/src/routes.rs` | mount |
| `packages/agent-workflows/mention/agent/lib/link.ts` | pure: HMAC + intent request shape |
| `packages/agent-workflows/mention/agent/channels/slack.ts:36` | `post` → `postEphemeral` |
| `tests/e2e/tests/slack_link_test.rs` | the two load-bearing tests |

---

## Task 1: The `temper-auth` crate (PR 1 — ONE atomic commit)

A cross-crate type move does not compile in halves. Every step below lands in a **single commit**. No behavior change: the CLI's existing login tests are the regression net.

**Files:**
- Create: `crates/temper-auth/Cargo.toml`, `src/lib.rs`, `src/pkce.rs`, `src/authorize.rs`, `src/token.rs`
- Modify: `crates/temper-client/src/login.rs` (removes `:41-51`, `:58-88`, `:94-104`; rewrites `:161`, `:360`, `:382`), `crates/temper-client/Cargo.toml`

**Interfaces:**
- Produces:
  - `temper_auth::generate_pkce_pair() -> (String, String)` — `(verifier, challenge)`
  - `temper_auth::AuthorizeParams { authorize_url: String, client_id: String, audience: Option<String>, redirect_uri: String, scopes: Vec<String>, state: String, code_challenge: String }`
  - `temper_auth::build_authorize_url(params: &AuthorizeParams) -> Result<String, AuthError>`
  - `temper_auth::TokenResponse { access_token: String, id_token: Option<String>, refresh_token: Option<String>, expires_in: Option<u64> }` — all fields `pub`
  - `temper_auth::AuthError::InvalidAuthorizeUrl(String)`

**Why `state: String` and not `port: u16`:** `login.rs:60` bakes the loopback port into the signature and writes it to `state` at `:80`. That is the single biggest reuse blocker — a server-side flow has no port. The CLI now stringifies its own port at the call site.

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-auth/src/authorize.rs` tests (write the test module first; the impl lands in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> AuthorizeParams {
        AuthorizeParams {
            authorize_url: "https://id.example.com/authorize".to_string(),
            client_id: "cid".to_string(),
            audience: Some("https://api.example.com".to_string()),
            redirect_uri: "https://temperkb.io/api/auth/slack/callback".to_string(),
            scopes: vec!["openid".to_string(), "offline_access".to_string()],
            state: "opaque-nonce".to_string(),
            code_challenge: "chal".to_string(),
        }
    }

    #[test]
    fn builds_url_with_opaque_state_and_s256() {
        let url = build_authorize_url(&params()).unwrap();
        assert!(url.contains("state=opaque-nonce"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("audience=https%3A%2F%2Fapi.example.com"));
        assert!(url.contains("scope=openid+offline_access"));
    }

    #[test]
    fn audience_is_omitted_when_absent() {
        let mut p = params();
        p.audience = None;
        let url = build_authorize_url(&p).unwrap();
        assert!(!url.contains("audience="));
    }

    #[test]
    fn malformed_authorize_url_is_an_error_not_a_panic() {
        let mut p = params();
        p.authorize_url = "not a url".to_string();
        assert!(matches!(
            build_authorize_url(&p),
            Err(AuthError::InvalidAuthorizeUrl(_))
        ));
    }
}
```

Create `crates/temper-auth/src/pkce.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use sha2::{Digest, Sha256};

    #[test]
    fn challenge_is_s256_of_verifier() {
        let (verifier, challenge) = generate_pkce_pair();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn verifier_is_43_chars_and_pairs_differ() {
        let (v1, _) = generate_pkce_pair();
        let (v2, _) = generate_pkce_pair();
        assert_eq!(v1.len(), 43);
        assert_ne!(v1, v2);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p temper-auth --all-features`
Expected: FAIL — `error: could not find `temper-auth` in workspace` (the crate does not exist yet).

- [ ] **Step 3: Write the crate**

`crates/temper-auth/Cargo.toml` (versions copied from `crates/temper-client/Cargo.toml`):

```toml
[package]
name = "temper-auth"
version = "0.1.0"
edition = "2021"
description = "Shared OAuth2 PKCE mechanics for temper's client and server surfaces"

[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
url = "2"
rand = "0.8"
sha2 = "0.10"
base64 = "0.22"
```

`crates/temper-auth/src/lib.rs`:

```rust
//! Shared OAuth2 Authorization Code + PKCE mechanics.
//!
//! Pure: crypto and string building, no HTTP and no I/O. Both surfaces need these —
//! temper-client for the CLI's loopback login, temper-services for the server-side
//! Slack account-link callback — and neither may depend on the other.
//!
//! What deliberately does NOT live here: the claims -> profile seam. `authenticate` /
//! `resolve_from_claims` are `pub(crate)` in temper-services *as a security property*
//! (a surface cannot hand them claims it built itself). Lifting them into a shared
//! crate would turn `pub(crate)` into `pub` across a crate boundary and the guarantee
//! would evaporate silently.

pub mod authorize;
pub mod pkce;
pub mod token;

pub use authorize::{build_authorize_url, AuthorizeParams};
pub use pkce::generate_pkce_pair;
pub use token::TokenResponse;

/// A fault in OAuth parameter construction.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    /// `authorize_url` came from configuration — a malformed value is a configuration
    /// fault, not a programming bug, so it propagates rather than panicking.
    #[error("authorize_url is not a valid URL ({0})")]
    InvalidAuthorizeUrl(String),
}
```

`crates/temper-auth/src/pkce.rs` (moved verbatim from `login.rs:41-51`; keep the tests from Step 1 below it):

```rust
//! PKCE (RFC 7636) S256 pair generation.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

/// Generate a PKCE `code_verifier` and its S256 `code_challenge`.
///
/// 32 random bytes -> a 43-character base64url verifier.
pub fn generate_pkce_pair() -> (String, String) {
    let random_bytes: [u8; 32] = rand::random();
    let verifier = URL_SAFE_NO_PAD.encode(random_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    (verifier, challenge)
}
```

`crates/temper-auth/src/authorize.rs` (keep the Step 1 tests below it):

```rust
//! Authorization-endpoint URL construction.

use crate::AuthError;

/// Inputs to an `/authorize` redirect. A struct rather than seven positional
/// parameters, per the repo's >5-parameter rule.
#[derive(Debug, Clone)]
pub struct AuthorizeParams {
    /// Authorization endpoint (e.g. `https://temperkb.us.auth0.com/authorize`).
    pub authorize_url: String,
    pub client_id: String,
    /// API audience, sent as the `audience` parameter. Omitted entirely when `None`.
    pub audience: Option<String>,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    /// Opaque to this crate. The CLI passes its loopback port; the Slack link flow
    /// passes a DB-backed single-use nonce.
    pub state: String,
    pub code_challenge: String,
}

/// Build the full authorization URL with PKCE parameters.
pub fn build_authorize_url(params: &AuthorizeParams) -> Result<String, AuthError> {
    let scope = params.scopes.join(" ");

    let mut url = url::Url::parse(&params.authorize_url)
        .map_err(|e| AuthError::InvalidAuthorizeUrl(format!("{}: {e}", params.authorize_url)))?;

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &params.client_id)
        .append_pair("redirect_uri", &params.redirect_uri)
        .append_pair("code_challenge", &params.code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &params.state)
        .append_pair("scope", &scope);

    if let Some(audience) = &params.audience {
        url.query_pairs_mut().append_pair("audience", audience);
    }

    Ok(url.to_string())
}
```

`crates/temper-auth/src/token.rs`:

```rust
//! The token-endpoint response wire type.

/// RFC 6749 token response. Shared so both surfaces deserialize the same shape.
///
/// `id_token` is carried but unused: the access token is what we persist and decode.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}
```

- [ ] **Step 4: Run to verify the new tests pass**

Run: `cargo test -p temper-auth --all-features`
Expected: PASS — 5 tests.

- [ ] **Step 5: Repoint temper-client**

In `crates/temper-client/Cargo.toml`, add under `[dependencies]`:

```toml
temper-auth = { path = "../temper-auth" }
```

In `crates/temper-client/src/login.rs`: **delete** `generate_pkce_pair` (`:40-51`), `build_authorize_url` (`:53-88`), and `struct TokenResponse` (`:94-104`). Add the import:

```rust
use temper_auth::{build_authorize_url, generate_pkce_pair, AuthorizeParams, TokenResponse};
```

Rewrite the `:161` call site — the port becomes the `state` string here, where it belongs:

```rust
    let auth_url = build_authorize_url(&AuthorizeParams {
        authorize_url: config.authorize_url.clone(),
        client_id: config.client_id.clone(),
        audience: config.audience.clone(),
        redirect_uri: config.callback_url.clone(),
        scopes: config.scopes.clone(),
        // The relay on temperkb.io reads the loopback port back out of `state`.
        state: port.to_string(),
        code_challenge: code_challenge.clone(),
    })
    .map_err(|e| crate::error::ClientError::NotConfigured(e.to_string()))?;
```

`exchange_code` (`:107`) keeps its body but returns the imported `TokenResponse`; its field reads are unchanged because the fields are now `pub`.

Rewrite the two in-crate tests at `:360` and `:382` to build an `AuthorizeParams` (mirroring the Step 1 helper) instead of passing `(&config, 12345, &challenge)`. Keep their assertions.

- [ ] **Step 6: Prune now-unused temper-client deps**

Run: `cargo make check`

If `cargo-machete` flags `rand`, `sha2`, or `base64` as unused in temper-client, remove those lines from `crates/temper-client/Cargo.toml`. If any is still used elsewhere in the crate, leave it. **Do not guess — let machete decide.**

- [ ] **Step 7: Verify the whole workspace, including the CLI's regression net**

Run: `cargo make check`
Expected: clean.

Run: `cargo test -p temper-client --all-features`
Expected: PASS — the pre-existing login tests are unchanged in intent and must still pass.

- [ ] **Step 8: Commit — ONE atomic commit**

```bash
git add crates/temper-auth crates/temper-client
git commit -m "refactor(auth): extract temper-auth — shared PKCE mechanics

A cross-crate type move, in one commit because it does not compile in halves.

temper-services needs generate_pkce_pair / build_authorize_url / TokenResponse
for T2's server-side Slack account-link callback. Reusing them from
temper-client would make temper-api depend on temper-CLIENT, inverting
server->client. So the pure mechanics move to a neutral crate both depend on.

build_authorize_url loses its baked-in port: u16 (the single biggest reuse
blocker — a server-side flow has no loopback port) and takes an AuthorizeParams
struct, per the >5-parameter rule. The CLI stringifies its own port into state
at the call site, where that concern belongs.

Scoped to what T2 needs. internal_sig and auth_config are NOT moved: T2
consumes them fine where they live, and the move would churn every existing
consumer for no gain.

No behavior change. The CLI's existing login tests are the regression net."
```

---

## Task 2: `kb_slack_link_intents` migration

**Files:**
- Create: `migrations/20260716000040_slack_link_intents.sql`

**Interfaces:**
- Produces: table `kb_slack_link_intents(id, state_nonce, code_verifier, slack_principal_id, expires_at, consumed_at, created_at)`

**Why a new table and not `kb_oauth_flow`:** that one is the AS's own bookkeeping with a `status CHECK IN ('pending_saml','code_issued','consumed')` (`migrations/20260701000006_saml_as_tables.sql:45-58`). Widening a shipped CHECK to carry client-side state would tangle the two halves of OAuth the spec separates. **Number 40**: `…010` (steward delta) and `…020` (backfill legacy profile emitters) are ALREADY TAKEN on main — verified against `git ls-tree origin/main` and the local `_sqlx_migrations`. `…030` is left free as the sibling gap. Verify the number is still free before applying; if it collides, renumber UP and never reset the database.

- [ ] **Step 1: Write the migration**

```sql
-- Slack account-link flow intents (T2; spec 2026-07-16-slack-account-link-design.md).
--
-- CLIENT-side OAuth state, deliberately distinct from `kb_oauth_flow`. That table is the
-- Authorization Server's own bookkeeping for flows IT authorizes (pending_saml -> code_issued
-- -> consumed). This one holds the PKCE verifier temper carries across a redirect while acting
-- as an OAuth *client* — of Auth0 on temperkb.io, of the local AS on an enterprise install.
-- Same protocol, opposite ends; one table each.
--
-- Additive. Namespace-free (no SET search_path).
--
-- `state_nonce` is opaque random, NOT a signed blob (spec D6): signed-and-stateless cannot be
-- single-use, and burning a state needs a store regardless. One mechanism therefore delivers
-- single-use, TTL and unguessability. Consume is an atomic conditional UPDATE ... RETURNING
-- (the `bindCodeToFlow` pattern, oauth/flow.ts:56-77): zero rows means unknown, expired or
-- replayed -- indistinguishably, and safely.

CREATE TABLE kb_slack_link_intents (
    id                 UUID PRIMARY KEY,
    -- The opaque `state` handed to the IdP and echoed back to the callback.
    state_nonce        TEXT NOT NULL UNIQUE,
    -- The PKCE verifier, held across the redirect. Paired with the challenge sent to the IdP.
    code_verifier      TEXT NOT NULL,
    -- The WHOLE opaque principal (`slack:<team>:<user>`). 2-4 segments; never split on ':'.
    slack_principal_id TEXT NOT NULL,
    expires_at         TIMESTAMPTZ NOT NULL,
    -- NULL until burned. The single-use marker.
    consumed_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The consume path filters unburned + unexpired rows by nonce; UNIQUE(state_nonce) already
-- indexes the lookup. This partial index serves reaping of abandoned intents.
CREATE INDEX idx_slack_link_intents_unconsumed
    ON kb_slack_link_intents (expires_at)
    WHERE consumed_at IS NULL;
```

- [ ] **Step 2: Apply and verify**

Run: `cargo make docker-up && sqlx migrate run`
Expected: the migration applies cleanly.

Verify:
```bash
psql "$DATABASE_URL" -c "\d kb_slack_link_intents"
```
Expected: the seven columns, `state_nonce` UNIQUE, `consumed_at` nullable.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260716000040_slack_link_intents.sql
git commit -m "feat(slack-link): kb_slack_link_intents — client-side OAuth flow state

Distinct from kb_oauth_flow, which is the AS's bookkeeping for flows IT
authorizes. This holds the PKCE verifier temper carries across a redirect as an
OAuth *client*. Same protocol, opposite ends.

state_nonce is opaque random rather than a signed blob: signed-and-stateless
cannot be single-use, and burning a state needs a store regardless."
```

---

## Task 3: `slack_link_service` — intent create/consume + link upsert

All SQL for this feature lives here. **Never inline `sqlx::query!()` in a handler.**

**Files:**
- Create: `crates/temper-services/src/services/slack_link_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs` (add `pub mod slack_link_service;`)

**Interfaces:**
- Consumes: `kb_slack_link_intents` (Task 2)
- Produces:
  - `create_intent(pool: &PgPool, slack_principal_id: &str, code_verifier: &str, ttl: Duration) -> ApiResult<String>` — returns the `state_nonce`
  - `consume_intent(pool: &PgPool, state_nonce: &str) -> ApiResult<Option<ConsumedIntent>>`
  - `pub struct ConsumedIntent { pub code_verifier: String, pub slack_principal_id: String }`
  - `upsert_slack_link(pool: &PgPool, profile_id: Uuid, slack_principal_id: &str) -> ApiResult<()>`
  - `pub const SLACK_AUTH_PROVIDER: &str = "slack";`

- [ ] **Step 1: Write the failing tests**

Append to `crates/temper-services/src/services/slack_link_service.rs`. **The `#[cfg(feature = "test-db")]` gate is mandatory** — a `#[sqlx::test]` module without it breaks the no-DB unit job.

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use std::time::Duration;

    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    #[sqlx::test]
    async fn consume_returns_the_verifier_and_principal_once(pool: PgPool) {
        let nonce = create_intent(&pool, PRINCIPAL, "verifier-abc", Duration::from_secs(600))
            .await
            .unwrap();

        let first = consume_intent(&pool, &nonce).await.unwrap().unwrap();
        assert_eq!(first.code_verifier, "verifier-abc");
        assert_eq!(first.slack_principal_id, PRINCIPAL);
    }

    /// The single-use invariant. A replayed state must not resolve.
    #[sqlx::test]
    async fn a_second_consume_of_the_same_nonce_yields_none(pool: PgPool) {
        let nonce = create_intent(&pool, PRINCIPAL, "verifier-abc", Duration::from_secs(600))
            .await
            .unwrap();

        assert!(consume_intent(&pool, &nonce).await.unwrap().is_some());
        assert!(consume_intent(&pool, &nonce).await.unwrap().is_none());
    }

    /// TTL. An expired intent is indistinguishable from an unknown one.
    #[sqlx::test]
    async fn an_expired_intent_yields_none(pool: PgPool) {
        let nonce = create_intent(&pool, PRINCIPAL, "v", Duration::from_secs(0))
            .await
            .unwrap();
        // ttl=0 => expires_at == now(); the `expires_at > now()` predicate excludes it.
        assert!(consume_intent(&pool, &nonce).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn an_unknown_nonce_yields_none(pool: PgPool) {
        assert!(consume_intent(&pool, "never-issued").await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn nonces_are_unique_across_intents(pool: PgPool) {
        let a = create_intent(&pool, PRINCIPAL, "v", Duration::from_secs(600)).await.unwrap();
        let b = create_intent(&pool, PRINCIPAL, "v", Duration::from_secs(600)).await.unwrap();
        assert_ne!(a, b);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db --lib slack_link`
Expected: FAIL — module not found.

- [ ] **Step 3: Write the service**

```rust
//! The Slack account-link flow's persistence: intent lifecycle + the directory row.
//!
//! All SQL for T2 lives here; the handlers dispatch and never touch the database.

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// The `auth_provider` value for every Slack link row. One constant so the write and
/// any future read can never disagree on the string.
pub const SLACK_AUTH_PROVIDER: &str = "slack";

/// What a successful consume yields: everything the callback needs to finish the exchange.
#[derive(Debug, Clone)]
pub struct ConsumedIntent {
    pub code_verifier: String,
    pub slack_principal_id: String,
}

/// Mint an intent and return its opaque `state_nonce`.
///
/// The nonce is a UUIDv7 rendered as text: unguessable, and time-sortable for reaping.
pub async fn create_intent(
    pool: &PgPool,
    slack_principal_id: &str,
    code_verifier: &str,
    ttl: Duration,
) -> ApiResult<String> {
    let state_nonce = Uuid::now_v7().to_string();
    let ttl_secs = ttl.as_secs() as f64;

    sqlx::query!(
        r#"
        INSERT INTO kb_slack_link_intents
            (id, state_nonce, code_verifier, slack_principal_id, expires_at)
        VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))
        "#,
        Uuid::now_v7(),
        state_nonce,
        code_verifier,
        slack_principal_id,
        ttl_secs,
    )
    .execute(pool)
    .await?;

    Ok(state_nonce)
}

/// Burn an intent and return its payload — atomically, exactly once.
///
/// The conditional UPDATE is the whole single-use mechanism: two concurrent callbacks race
/// on the same row and exactly one sees `consumed_at IS NULL`. `None` means unknown, expired
/// OR replayed — indistinguishably, which is the point. The caller must not try to tell them
/// apart, and must not say which it was.
pub async fn consume_intent(pool: &PgPool, state_nonce: &str) -> ApiResult<Option<ConsumedIntent>> {
    let row = sqlx::query!(
        r#"
        UPDATE kb_slack_link_intents
           SET consumed_at = now()
         WHERE state_nonce = $1
           AND consumed_at IS NULL
           AND expires_at > now()
        RETURNING code_verifier, slack_principal_id
        "#,
        state_nonce,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ConsumedIntent {
        code_verifier: r.code_verifier,
        slack_principal_id: r.slack_principal_id,
    }))
}

/// Write the directory row `slack:<team>:<user> -> profile`.
///
/// Idempotent on re-link via `UNIQUE(auth_provider, auth_provider_user_id)`. A conflict that
/// carries a DIFFERENT profile_id is a rebind, and it is allowed by design: binding requires
/// authenticating AS the target profile, so a principal can only ever bind to the
/// authenticator's own profile. See spec D4.
///
/// `email` stays NULL: Slack supplies no email on the wire, which is exactly why the link is
/// keyed on the opaque principal.
pub async fn upsert_slack_link(
    pool: &PgPool,
    profile_id: Uuid,
    slack_principal_id: &str,
) -> ApiResult<()> {
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (auth_provider, auth_provider_user_id)
        DO UPDATE SET profile_id = EXCLUDED.profile_id, linked_at = now()
        "#,
        Uuid::now_v7(),
        profile_id,
        SLACK_AUTH_PROVIDER,
        slack_principal_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --lib slack_link`
Expected: PASS — 5 tests.

- [ ] **Step 5: Commit (offline cache is stale until Task 10)**

```bash
git add crates/temper-services/src/services/slack_link_service.rs crates/temper-services/src/services/mod.rs
git commit --no-verify -m "feat(slack-link): intent lifecycle + link upsert service

The conditional UPDATE ... RETURNING is the whole single-use mechanism: two
concurrent callbacks race and exactly one sees consumed_at IS NULL. None means
unknown, expired OR replayed -- indistinguishably, which is the point.

--no-verify: new query! macros fail offline until the cache regen in Task 10."
```

---

## Task 4: Lookup-only profile resolution

**The load-bearing security task.** `authenticate_token` is a *login* path that auto-provisions, and the profile INSERT alone fires `trg_sync_system_membership` (`migrations/20260624000002_canonical_functions.sql:79-81`) → `ensure_auto_join_memberships` → in `open` mode (**production**) the new profile joins **every** auto-join team. A stray Slack click must never confer reach no approved auth flow backs.

**Files:**
- Modify: `crates/temper-services/src/services/profile_service.rs` (near `:117`)
- Modify: `crates/temper-services/src/auth/mod.rs` (near `:95`)

**Interfaces:**
- Produces:
  - `temper_services::auth::authenticate_token_existing_only(state: &AppState, raw: &RawJwtClaims, token: &str) -> Result<AuthenticatedProfile, AuthzError>`
  - `profile_service::resolve_existing_human_from_claims(pool, claims) -> ApiResult<Option<Profile>>` (`pub(crate)`)

**The seam is preserved.** The new entry point takes `RawJwtClaims` + the raw token exactly as `authenticate_token` does, and builds the `AuthClaims` internally. Nothing outside temper-services can hand it claims. Do **not** make `AuthClaims` construction reachable from a surface.

**Steps 1–4 keep working; only step 5 (create) is refused.** `reconcile_by_email` attaches a new link to an **existing** profile — it creates no profile, fires no trigger, and confers no reach. Refusing it would make Slack-connect behave differently from every other login for no safety gain.

- [ ] **Step 1: Write the failing tests**

In `crates/temper-services/src/services/profile_service.rs`, inside the existing `#[cfg(all(test, feature = "test-db"))]` test module (match the surrounding helpers — reuse the module's existing claims builder rather than inventing one):

```rust
    /// The D3 invariant. Asserting the None alone would not catch a regression that
    /// creates the profile and THEN errors, so assert the absence of the row too.
    #[sqlx::test]
    async fn lookup_only_refuses_an_unknown_sub_and_creates_no_profile(pool: PgPool) {
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .unwrap();

        let claims = human_claims("auth0|never-seen-before", "nobody@example.com");
        let resolved = resolve_existing_human_from_claims(&pool, &claims).await.unwrap();
        assert!(resolved.is_none(), "an unknown sub must not resolve");

        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, after, "lookup-only must not mint a profile");
    }

    #[sqlx::test]
    async fn lookup_only_resolves_an_existing_linked_profile(pool: PgPool) {
        let claims = human_claims("auth0|existing", "someone@example.com");
        // The normal login path mints it once...
        let created = resolve_from_claims(&pool, &claims).await.unwrap();

        // ...and lookup-only then finds that same profile.
        let found = resolve_existing_human_from_claims(&pool, &claims)
            .await
            .unwrap()
            .expect("an existing link must resolve");
        assert_eq!(found.id, created.id);
    }

    /// A machine-shaped identity is refused here as it is everywhere else.
    #[sqlx::test]
    async fn lookup_only_refuses_a_machine_shaped_identity(pool: PgPool) {
        let claims = human_claims("abc123@clients", "nobody@example.com");
        assert!(resolve_existing_human_from_claims(&pool, &claims).await.is_err());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db --lib profile_service`
Expected: FAIL — `cannot find function resolve_existing_human_from_claims`.

- [ ] **Step 3: Implement the narrowing**

In `profile_service.rs`, add beside `resolve_human_from_claims`:

```rust
/// Human path, LOOKUP-ONLY: link lookup -> email reconcile -> **refuse**.
///
/// `resolve_human_from_claims`'s steps 1-4 verbatim, minus step 5's create. It exists because
/// connecting Slack is NOT a registration route: `create_new_profile_and_link` INSERTs
/// `kb_profiles`, and that INSERT alone fires `trg_sync_system_membership` ->
/// `ensure_auto_join_memberships`, which in `open` mode (production's default) joins the new
/// profile to EVERY auto-join team. That reach would be backed by no approved auth flow. There
/// is no way to create the profile without it — the enrollment is a trigger, not a decision.
///
/// Steps 3-4 (`reconcile_by_email`) are KEPT: they attach a link to an EXISTING profile,
/// creating nothing and firing no trigger. Refusing them would make Slack-connect behave
/// differently from every other login for no safety gain.
///
/// Mirrors the machine arm, which is already lookup-or-reject (`resolve_machine_from_claims`).
/// `Ok(None)` means "no such profile" — the caller renders one indistinguishable refusal.
pub(crate) async fn resolve_existing_human_from_claims(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<Option<Profile>> {
    // 0: the machine-shape guard, identical to the create path's. A narrowing must not
    // become a hole: a caller must not walk a machine identity past the registration gate
    // by choosing the lookup-only door.
    let machine_provider = claims.provider == crate::auth::MACHINE_PROVIDER_TAG;
    let machine_shaped_id = claims.external_user_id.ends_with("@clients");
    if machine_provider || machine_shaped_id {
        tracing::warn!(
            external_user_id = %claims.external_user_id,
            provider = %claims.provider,
            "machine gate: rejected (machine-shaped identity on the human lookup-only path)"
        );
        return Err(ApiError::Unauthorized(format!(
            "identity '{}' is machine-shaped and cannot resolve to a human profile.",
            claims.external_user_id
        )));
    }

    // 1 & 2: direct lookup by provider + external user id.
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        refresh_link_verification(pool, &link, claims).await?;
        return get_by_id(pool, ProfileId::from(link.profile_id)).await.map(Some);
    }

    // 3 & 4: email reconciliation — attaches to an EXISTING profile; mints nothing.
    if let Some(profile) = reconcile_by_email(pool, claims).await? {
        return Ok(Some(profile));
    }

    // 5: deliberately absent. No create branch.
    Ok(None)
}
```

In `auth/mod.rs`, extract the claims construction from `authenticate_token` (`:95-122`) into a private helper and add the second entry point. `authenticate_token`'s observable behavior must not change:

```rust
/// The classify + human-email-ladder block, shared by both token entry points.
///
/// The machine arm must NOT run the email ladder: an M2M principal has no email and no
/// `/userinfo` to ask, so a ladder on that path would be an authentication failure dressed
/// as a lookup. That ordering is load-bearing — see
/// `machine_token_authenticates_without_running_the_email_ladder`.
async fn claims_from_token(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthClaims, AuthzError> {
    match classify(raw) {
        Principal::Machine(machine) => Ok(machine),
        Principal::Refuse(why) => {
            tracing::warn!(sub = %raw.sub, why, "rejected: unclassifiable machine-shaped token");
            Err(AuthzError::Refused(why))
        }
        Principal::Human => {
            let (email, email_verified) = email::resolve_email_from_claims(state, raw, token)
                .await
                .map_err(AuthzError::EmailResolution)?;
            Ok(AuthClaims {
                principal_kind: PrincipalKind::Human,
                provider: state.config.auth_provider_name.clone(),
                external_user_id: raw.sub.clone(),
                email,
                email_verified,
                exp: raw.exp,
                iat: raw.iat,
            })
        }
    }
}

/// **The token path, LOOKUP-ONLY.** A verified JWT ⇒ an *existing* profile, or `Unauthorized`.
///
/// Same seam as [`authenticate_token`] — it takes raw claims and the bearer, and builds the
/// `AuthClaims` itself, so no surface can hand it forged ones. It differs in exactly one way:
/// it never provisions.
///
/// Used by the Slack account-link callback. Connecting Slack is not a registration route: the
/// profile INSERT auto-provisions fires `trg_sync_system_membership`, which in `open` mode joins
/// EVERY auto-join team. See the T2 spec, D3.
pub async fn authenticate_token_existing_only(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthenticatedProfile, AuthzError> {
    let claims = claims_from_token(state, raw, token).await?;

    let profile = profile_service::resolve_existing_human_from_claims(&state.pool, &claims)
        .await
        .map_err(AuthzError::from)?
        .ok_or_else(|| {
            tracing::info!(sub = %raw.sub, "slack link: refused (no existing temper profile)");
            AuthzError::Refused("no existing temper profile for this identity")
        })?;

    // Level 1's remaining gates apply unchanged: an inactive or access-denied profile is
    // refused here exactly as it is on the login path.
    gate_resolved_profile(&state.pool, profile).await
}
```

> **Implementer note:** `authenticate_token` currently inlines the `match classify(raw)` block and then calls `authenticate(&state.pool, &claims)`. Rewrite it to `let claims = claims_from_token(state, raw, token).await?; authenticate(&state.pool, &claims).await` — no behavior change. Read `authenticate` (`:165` onward) to see which post-resolution gates it applies (active check, system access) and factor the shared tail into `gate_resolved_profile` so **both** paths run identical gates. If `authenticate`'s tail is not cleanly separable, call the gates directly in `authenticate_token_existing_only` rather than duplicating their logic — but they MUST run. A lookup-only path that skips the active/system-access gates would be a hole, not a narrowing.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db --lib profile_service`
Expected: PASS.

Run: `cargo nextest run -p temper-services --features test-db --lib auth`
Expected: PASS — every pre-existing auth test, unchanged. `machine_token_authenticates_without_running_the_email_ladder` passing proves the refactor preserved the ordering.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/services/profile_service.rs crates/temper-services/src/auth/mod.rs
git commit --no-verify -m "feat(slack-link): lookup-only human resolution

Connecting Slack is not a registration route. create_new_profile_and_link
INSERTs kb_profiles, and that INSERT alone fires trg_sync_system_membership ->
ensure_auto_join_memberships, which in open mode (production's default) joins
the new profile to EVERY auto-join team. That reach would be backed by no
approved auth flow, and there is no way to create the profile without it -- the
enrollment is a trigger, not a decision.

Steps 1-4 are kept; only step 5's create is refused. reconcile_by_email attaches
to an EXISTING profile, mints nothing and fires no trigger.

The seam holds: the new entry point takes RawJwtClaims + the bearer and builds
AuthClaims itself, exactly as authenticate_token does."
```

---

## Task 5: Provider derivation + token exchange

**Files:**
- Create: `crates/temper-services/src/link_provider.rs`, `crates/temper-services/src/oauth_client.rs`
- Modify: `crates/temper-services/src/lib.rs` (add both `pub mod`s), `crates/temper-services/src/config.rs`, `crates/temper-services/Cargo.toml`

**Interfaces:**
- Consumes: `temper_auth::{AuthorizeParams, build_authorize_url, TokenResponse}` (Task 1); `AuthConfig { issuer, mode }` from `crates/temper-services/src/auth_config.rs`
- Produces:
  - `link_provider::LinkProvider { pub authorize_url: String, pub token_url: String, pub client_id: String, pub redirect_uri: String }`
  - `link_provider::derive(auth: &AuthConfig, cfg: &SlackLinkConfig) -> LinkProvider`
  - `config::SlackLinkConfig { pub client_id: String, pub hmac_secret: String, pub public_base_url: String }`
  - `ApiConfig.slack_link: Option<SlackLinkConfig>`
  - `oauth_client::exchange_code(token_url: &str, client_id: &str, code: &str, code_verifier: &str, redirect_uri: &str) -> ApiResult<TokenResponse>`

- [ ] **Step 1: Write the failing tests**

`crates/temper-services/src/link_provider.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_config::{AuthConfig, AuthMode};

    fn slack_cfg() -> crate::config::SlackLinkConfig {
        crate::config::SlackLinkConfig {
            client_id: "slack-link-client".to_string(),
            hmac_secret: "s3cret".to_string(),
            public_base_url: "https://temperkb.io".to_string(),
        }
    }

    fn auth(issuer: &str, mode: AuthMode) -> AuthConfig {
        AuthConfig {
            issuer: issuer.to_string(),
            jwks_url: "https://unused/.well-known/jwks.json".to_string(),
            audience: "https://api.temperkb.io".to_string(),
            mode,
        }
    }

    #[test]
    fn external_idp_points_at_the_idp_domain() {
        let p = derive(&auth("https://temperkb.us.auth0.com/", AuthMode::ExternalIdp), &slack_cfg());
        assert_eq!(p.authorize_url, "https://temperkb.us.auth0.com/authorize");
        assert_eq!(p.token_url, "https://temperkb.us.auth0.com/oauth/token");
    }

    /// A trailing slash on the issuer must not produce a doubled one.
    #[test]
    fn external_idp_tolerates_a_missing_trailing_slash() {
        let p = derive(&auth("https://temperkb.us.auth0.com", AuthMode::ExternalIdp), &slack_cfg());
        assert_eq!(p.authorize_url, "https://temperkb.us.auth0.com/authorize");
    }

    /// AS mode: the endpoints live on the instance itself, not a separate auth host.
    #[test]
    fn temper_as_points_at_the_instance_itself() {
        let p = derive(&auth("https://temper.acme.com", AuthMode::TemperAs), &slack_cfg());
        assert_eq!(p.authorize_url, "https://temper.acme.com/oauth/authorize");
        assert_eq!(p.token_url, "https://temper.acme.com/oauth/token");
    }

    #[test]
    fn redirect_uri_is_the_public_callback() {
        let p = derive(&auth("https://x/", AuthMode::ExternalIdp), &slack_cfg());
        assert_eq!(p.redirect_uri, "https://temperkb.io/api/auth/slack/callback");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p temper-services --all-features link_provider`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

`crates/temper-services/src/link_provider.rs`:

```rust
//! Mode-aware OAuth endpoint derivation for the Slack account-link flow.
//!
//! Temper is an OAuth *client* here, of whichever issuer fronts this instance:
//! Auth0 on temperkb.io, the co-deployed AS on an enterprise install. `AuthConfig.mode`
//! already carries which, so this is derivation, not new configuration.

use crate::auth_config::{AuthConfig, AuthMode};
use crate::config::SlackLinkConfig;

/// The resolved endpoints for one link flow.
#[derive(Debug, Clone)]
pub struct LinkProvider {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub redirect_uri: String,
}

/// The callback path. Public (browser-facing), so it is served by the axum function via
/// vercel.json's `/(.*)` catch-all — the `filesystem` handler finds no file at this path.
pub const CALLBACK_PATH: &str = "/api/auth/slack/callback";

/// Derive the endpoints from the instance's auth identity.
///
/// AS mode mirrors `temper-cli`'s `Idp::TemperAs`: the endpoints live on the instance
/// itself rather than a separate auth host, so temper-api exchanges against its own
/// deployment's `/oauth/token`. That self-hop is not a wart — it is what any OAuth client
/// colocated with its AS does, and it keeps ONE code path across both modes.
pub fn derive(auth: &AuthConfig, cfg: &SlackLinkConfig) -> LinkProvider {
    let base = auth.issuer.trim_end_matches('/');

    let (authorize_url, token_url) = match auth.mode {
        AuthMode::ExternalIdp => (format!("{base}/authorize"), format!("{base}/oauth/token")),
        AuthMode::TemperAs => (
            format!("{base}/oauth/authorize"),
            format!("{base}/oauth/token"),
        ),
    };

    LinkProvider {
        authorize_url,
        token_url,
        client_id: cfg.client_id.clone(),
        redirect_uri: format!(
            "{}{CALLBACK_PATH}",
            cfg.public_base_url.trim_end_matches('/')
        ),
    }
}
```

`crates/temper-services/src/oauth_client.rs`:

```rust
//! The token-endpoint exchange. The only outbound HTTP in temper-services.
//!
//! Deliberately not shared with temper-client's copy: sharing it would mean either putting
//! reqwest in the neutral crate (bloating the CLI) or inverting the server->client dependency.
//! The wire TYPE is shared (`temper_auth::TokenResponse`); ~20 lines of form POST are not.

use temper_auth::TokenResponse;

use crate::error::{ApiError, ApiResult};

/// Exchange an authorization code for tokens (RFC 6749 §4.1.3) with PKCE.
///
/// Never logs the code, the verifier, or any token.
pub async fn exchange_code(
    token_url: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> ApiResult<TokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("token exchange transport failure: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // The IdP's body can echo request parameters; log the status only.
        tracing::warn!(%status, "slack link: token exchange rejected by the IdP");
        return Err(ApiError::Unauthorized("token exchange failed".to_string()));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| ApiError::Internal(format!("token response was not the expected shape: {e}")))
}
```

In `crates/temper-services/src/config.rs`, add the struct and wire it into `ApiConfig`, following the `parse_vercel_connect` all-or-nothing precedent:

```rust
/// Slack account-link configuration. `None` when the three values are not all present —
/// a partial set is treated as unconfigured, so the endpoints are disabled rather than
/// half-configured (the `parse_vercel_connect` precedent).
#[derive(Debug, Clone)]
pub struct SlackLinkConfig {
    /// The OAuth client the link flow authorizes as. Its redirect_uri must be registered:
    /// Auth0's Allowed Callback URLs, or `AS_CLIENTS` on an AS instance.
    pub client_id: String,
    /// Shared with the mention agent; gates `POST /internal/slack/link-intents`.
    /// Distinct from `INTERNAL_RECONCILE_SECRET`: a different principal gets a different secret.
    pub hmac_secret: String,
    /// This instance's public origin, used to build the callback redirect_uri.
    pub public_base_url: String,
}

fn parse_slack_link(lookup: impl Fn(&str) -> Option<String>) -> Option<SlackLinkConfig> {
    let get = |k| lookup(k).filter(|s: &String| !s.is_empty());
    Some(SlackLinkConfig {
        client_id: get("SLACK_LINK_CLIENT_ID")?,
        hmac_secret: get("SLACK_LINK_SECRET")?,
        public_base_url: get("PUBLIC_BASE_URL")?,
    })
}
```

Add `pub slack_link: Option<SlackLinkConfig>,` to `ApiConfig` (documented like its siblings) and `slack_link: parse_slack_link(&lookup),` to the `Ok(Self { ... })` block. Add `pub mod link_provider;` and `pub mod oauth_client;` to `crates/temper-services/src/lib.rs`, and `temper-auth = { path = "../temper-auth" }` to `crates/temper-services/Cargo.toml`.

> **Implementer note:** `crates/temper-services/src/auth/mod.rs:238` builds an `ApiConfig` for tests with `internal_reconcile_secret: None`. Adding a field breaks that literal — add `slack_link: None` there. Compile errors will point at every other such site; fix them all the same way.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p temper-services --all-features link_provider`
Expected: PASS — 4 tests.

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/link_provider.rs crates/temper-services/src/oauth_client.rs crates/temper-services/src/config.rs crates/temper-services/src/lib.rs crates/temper-services/Cargo.toml crates/temper-services/src/auth/mod.rs
git commit --no-verify -m "feat(slack-link): mode-aware provider derivation + token exchange

AuthConfig.mode already says which issuer fronts the instance, so this is
derivation rather than new configuration. AS mode mirrors the CLI's
Idp::TemperAs: endpoints on the instance itself, so temper-api exchanges against
its own deployment's /oauth/token. That self-hop is what any OAuth client
colocated with its AS does, and it keeps ONE code path across both modes.

SLACK_LINK_SECRET is deliberately distinct from INTERNAL_RECONCILE_SECRET: a
different principal gets a different secret."
```

---

## Task 6: The HMAC gate for link-intents

`require_internal_signature` reads `internal_reconcile_secret` specifically. The agent is a **different principal** and must present a **different secret**, so extract the verification and add a second thin gate. Same scheme, same freshness window, different key.

**Files:**
- Modify: `crates/temper-api/src/middleware/internal_auth.rs`

**Interfaces:**
- Consumes: `temper_core::internal_sig::{verify, timestamp_is_fresh, SIGNATURE_HEADER, TIMESTAMP_HEADER}`; `SlackLinkConfig.hmac_secret` (Task 5)
- Produces: `require_slack_link_signature(State<AppState>, Request<Body>, Next) -> Result<Response, ApiError>`

- [ ] **Step 1: Refactor the existing gate to take its secret**

Extract the body of `require_internal_signature` (`:28-95`) into a private helper, changing nothing about its behavior:

```rust
/// The shared gate: fresh timestamp + valid HMAC over the exact bytes received.
///
/// `secret` is passed in rather than read from state because two different principals use
/// this scheme with two different keys — the co-deployed AS (`INTERNAL_RECONCILE_SECRET`)
/// and the Slack mention agent (`SLACK_LINK_SECRET`). One scheme, one implementation, two
/// keys; a shared key would let either forge the other's calls.
async fn require_signature_with(
    secret: &str,
    label: &'static str,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    // ... body of the current require_internal_signature from `let (parts, body) = ...`
    // onward, with every `tracing::warn!("internal reconcile: ...")` becoming
    // `tracing::warn!("{label}: ...")`.
}
```

Then `require_internal_signature` becomes:

```rust
pub async fn require_internal_signature(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    // Fail-closed when unconfigured: no secret ⇒ endpoint disabled.
    let secret = match state.config.internal_reconcile_secret.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            tracing::warn!("internal reconcile: rejected (endpoint disabled — secret unset)");
            return Err(ApiError::Unauthorized(
                "internal reconcile disabled".to_string(),
            ));
        }
    };
    require_signature_with(&secret, "internal reconcile", request, next).await
}
```

- [ ] **Step 2: Add the slack-link gate**

```rust
/// Rejects the request unless it carries a fresh, valid HMAC over its body, keyed on
/// `SLACK_LINK_SECRET`.
///
/// This gate is what makes Slack-side hijack expensive. Slack user ids are VISIBLE in a
/// workspace, so an open intent endpoint would let anyone mint a link URL for any user's
/// principal, bind it to their own profile, and silently receive that user's future
/// @temper writes. The gate means the URL is only ever minted in response to a real
/// mention and delivered ephemerally — the attacker must steal a message only the victim
/// can see. See the T2 spec, D5.
///
/// Fail-closed: no secret ⇒ endpoint disabled.
pub async fn require_slack_link_signature(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let secret = match state.config.slack_link.as_ref().map(|c| c.hmac_secret.clone()) {
        Some(s) if !s.is_empty() => s,
        _ => {
            tracing::warn!("slack link: rejected (endpoint disabled — SLACK_LINK_SECRET unset)");
            return Err(ApiError::Unauthorized("slack link disabled".to_string()));
        }
    };
    require_signature_with(&secret, "slack link", request, next).await
}
```

- [ ] **Step 3: Verify the existing gate is unchanged**

Run: `cargo make check`
Expected: clean.

Run: `cargo nextest run -p temper-api --features test-db --test internal_saml_test`
Expected: PASS. If no such test binary exists, run `ls crates/temper-api/tests/` and run whichever binary covers the reconcile gate. **Do not skip this** — the refactor touches a live auth gate, and its existing tests are the only proof it still behaves.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/middleware/internal_auth.rs
git commit -m "feat(slack-link): HMAC gate keyed on SLACK_LINK_SECRET

One scheme, one implementation, two keys. The agent is a different principal
from the co-deployed AS, so it gets a different secret -- a shared key would let
either forge the other's calls.

The gate is what makes Slack-side hijack expensive: Slack user ids are visible
in a workspace, so an open intent endpoint would let anyone mint a link URL for
any user's principal. With it, the attacker must steal an ephemeral message."
```

---

## Task 7: Both handlers + routes

**Files:**
- Create: `crates/temper-api/src/handlers/slack_link.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`, `crates/temper-api/src/routes.rs`

**Interfaces:**
- Consumes: everything from Tasks 3–6
- Produces: `slack_link::create_link_intent`, `slack_link::callback`

**Both routes are mounted with plain `.route()` (no `#[utoipa::path]`)**, like `/internal/saml/reconcile` and `/api/access/admin/*` — they are not part of the documented user API and must stay out of OpenAPI, or they restale `openapi.json` and three generated SDKs.

**Routing:** `/internal/slack/link-intents` matches vercel.json's `/internal/(.*)` → the internal function. `/api/auth/slack/callback` has no file at that path, so the `filesystem` handler misses and `/(.*)` sends it to axum. **No vercel.json change is needed** — both functions share the database, which is where the intent lives.

- [ ] **Step 1: Write the handlers**

```rust
//! The Slack account-link flow. Two endpoints, two audiences.
//!
//! `create_link_intent` is server-to-server (the mention agent, HMAC-gated). `callback` is
//! browser-facing and renders HTML, never JSON — a human is looking at it.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;

use temper_auth::{build_authorize_url, generate_pkce_pair, AuthorizeParams};
use temper_services::error::ApiError;
use temper_services::services::slack_link_service;
use temper_services::state::AppState;
use temper_services::{link_provider, oauth_client};

/// How long a link URL stays usable. Long enough for a human to notice the ephemeral
/// message and finish a browser login; short enough to bound a stolen one.
const INTENT_TTL: Duration = Duration::from_secs(15 * 60);

/// The scopes the link grant requests. `offline_access` is what makes the exchange return a
/// refresh token — T2 obtains the grant; T3 vaults it.
const LINK_SCOPES: [&str; 2] = ["openid", "offline_access"];

#[derive(Debug, serde::Deserialize)]
pub struct CreateLinkIntentRequest {
    /// The WHOLE opaque principal from `attributes` — 2-4 segments, never split.
    pub slack_principal_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct CreateLinkIntentResponse {
    pub authorize_url: String,
}

/// `POST /internal/slack/link-intents` — mint a PKCE pair + opaque state, return the IdP URL.
///
/// Gated by `require_slack_link_signature`. The signature covers THIS call, not the URL the
/// user later clicks: `internal_sig`'s skew window is 30s and a human clicks minutes later,
/// so signing the user-facing URL would force us to loosen a gate that is tight for good
/// reason. What the user receives is the IdP's own authorize URL with an opaque state.
pub async fn create_link_intent(
    State(state): State<AppState>,
    Json(req): Json<CreateLinkIntentRequest>,
) -> Result<Json<CreateLinkIntentResponse>, ApiError> {
    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;
    let provider = link_provider::derive(&state.config.auth, cfg);

    let (verifier, challenge) = generate_pkce_pair();
    let state_nonce = slack_link_service::create_intent(
        &state.pool,
        &req.slack_principal_id,
        &verifier,
        INTENT_TTL,
    )
    .await?;

    let authorize_url = build_authorize_url(&AuthorizeParams {
        authorize_url: provider.authorize_url,
        client_id: provider.client_id,
        audience: Some(state.config.auth.audience.clone()),
        redirect_uri: provider.redirect_uri,
        scopes: LINK_SCOPES.iter().map(|s| s.to_string()).collect(),
        state: state_nonce,
        code_challenge: challenge,
    })
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CreateLinkIntentResponse { authorize_url }))
}

#[derive(Debug, serde::Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// `GET /api/auth/slack/callback` — the registered redirect_uri.
///
/// Renders HTML on every path. Never JSON, and never a redirect back to Slack: the human is
/// already looking at this page, so it IS the confirmation. temper-api holds no Slack
/// credential and knows no channel.
pub async fn callback(State(state): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    match run_callback(&state, q).await {
        Ok(slug) => page(
            "✅ Connected",
            &format!("Linked as <strong>@{}</strong>. You can close this tab and go back to Slack."
                , html_escape(&slug)),
        ),
        Err(message) => page("Not connected", &html_escape(&message)),
    }
}

/// The flow proper. Returns the linked profile's slug, or a user-actionable message.
///
/// `Profile` has NO `handle` field — it is `slug` (`temper-core/src/types/profile.rs:23`).
/// The `handle` on the team types is a different thing; do not reach for it.
///
/// Every `Err` string here is shown to a human, so none of them may reveal whether a given
/// profile exists.
async fn run_callback(state: &AppState, q: CallbackQuery) -> Result<String, String> {
    if let Some(err) = q.error {
        tracing::warn!(error = %err, "slack link: IdP returned an error");
        return Err("The sign-in was cancelled or refused. Mention @temper again to retry.".into());
    }

    let (Some(code), Some(state_nonce)) = (q.code, q.state) else {
        return Err("That link is incomplete. Mention @temper again to get a fresh one.".into());
    };

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| "Account linking is not configured on this instance.".to_string())?;
    let provider = link_provider::derive(&state.config.auth, cfg);

    // Single-use + TTL + unguessability, in one atomic burn. Unknown, expired and replayed
    // are indistinguishable here BY DESIGN — do not try to tell the user which it was.
    let intent = slack_link_service::consume_intent(&state.pool, &state_nonce)
        .await
        .map_err(|_| "Something went wrong. Mention @temper again to retry.".to_string())?
        .ok_or_else(|| {
            tracing::warn!("slack link: rejected (unknown, expired or replayed state)");
            "That link has expired or was already used. Mention @temper again to get a fresh one."
                .to_string()
        })?;

    let tokens = oauth_client::exchange_code(
        &provider.token_url,
        &provider.client_id,
        &code,
        &intent.code_verifier,
        &provider.redirect_uri,
    )
    .await
    .map_err(|_| "Sign-in could not be completed. Mention @temper again to retry.".to_string())?;

    // LOOKUP-ONLY. Connecting Slack is not a registration route (spec D3).
    let profile = resolve_existing(state, &tokens.access_token).await.map_err(|_| {
        "No temper account is linked to this login. Sign in at temperkb.io first, then \
         mention @temper again to connect."
            .to_string()
    })?;

    // Auth before write: the profile is resolved and gated above this line.
    slack_link_service::upsert_slack_link(&state.pool, profile.profile.id, &intent.slack_principal_id)
        .await
        .map_err(|_| "Something went wrong saving the link. Mention @temper again.".to_string())?;

    // T3 SEAM. The exchange requested `offline_access`, so `tokens.refresh_token` is the
    // per-user grant -- its own independent family, never an export of the user's CLI grant.
    // T3 encrypts and stores it here, keyed by slack_principal_id, and adds refresh.
    // T2 deliberately does not persist it: identity (the row above) and secret (T3's vault)
    // stay in separate tables, and kb_profile_auth_links must never grow a secret column.
    tracing::info!(
        profile_id = %profile.profile.id,
        grant_received = tokens.refresh_token.is_some(),
        "slack link: established",
    );

    Ok(profile.profile.slug.clone())
}

fn page(title: &str, body: &str) -> Response {
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title} · temper</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;line-height:1.5\">\
         <h1>{title}</h1><p>{body}</p></body></html>"
    ))
    .into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
```

> **Implementer note — `resolve_existing`:** `authenticate_token_existing_only` (Task 4) needs `RawJwtClaims`, which means verifying the freshly-received access token's signature first. Read `crates/temper-api/src/middleware/auth.rs:49-75` and reuse **exactly** that JWKS path — `state.jwks_store.get_decoding_key()`, `state.jwks_store.validation(issuer, audience, alg)`, `jsonwebtoken::decode::<RawJwtClaims>`. Write `resolve_existing(state, access_token) -> Result<AuthenticatedProfile, ApiError>` as a private fn in this module that does that decode and then calls `authenticate_token_existing_only`. **Verify the signature — do not trust the token because it came from the exchange.** A token we did not verify is a token we did not authenticate, whatever channel delivered it.

`AuthenticatedProfile` is `{ profile: Profile, claims: AuthClaims }` (`temper-core/src/types/auth.rs:63-66`), hence the `profile.profile.slug` double-hop — that is correct, not a typo.

- [ ] **Step 2: Mount the routes**

In `crates/temper-api/src/routes.rs`, add to `internal_routes()` (near `:238`):

```rust
        .route(
            "/internal/slack/link-intents",
            post(handlers::slack_link::create_link_intent),
        )
```

> **Implementer note:** `internal_routes()` is layered with `require_internal_signature` at `:293` and `:329`. That is the WRONG secret for this route. Either give the slack-link route its own `Router` layered with `require_slack_link_signature` and merge it in both `create_app` and `create_internal_app`, or apply the gate per-route with `route_layer`. **Read `:270-340` and follow whichever the existing structure makes cleaner — but the reconcile secret must not gate this route, and this route must not be reachable ungated.**

Add a public router for the callback, mounted in `create_app` only (it is browser-facing; the internal function never serves it):

```rust
        .route(
            "/api/auth/slack/callback",
            get(handlers::slack_link::callback),
        )
```

Add `pub mod slack_link;` to `crates/temper-api/src/handlers/mod.rs` and `temper-auth = { path = "../temper-auth" }` to `crates/temper-api/Cargo.toml`.

- [ ] **Step 3: Verify**

Run: `cargo make check`
Expected: clean.

Run: `cargo make openapi-check`
Expected: PASS with **no diff** — these routes use plain `.route()` and must not enter the spec. If `openapi.json` changed, a `#[utoipa::path]` leaked in; remove it.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/handlers/slack_link.rs crates/temper-api/src/handlers/mod.rs crates/temper-api/src/routes.rs crates/temper-api/Cargo.toml
git commit --no-verify -m "feat(slack-link): the intent + callback endpoints

The callback renders HTML on every path, never JSON: a human is looking at it,
and that page IS the confirmation. temper-api holds no Slack credential and
knows no channel.

Unknown, expired and replayed states are indistinguishable to the user by
design. The 'no temper account' refusal is worded identically whether the
account is absent or something else failed."
```

---

## Task 8: The agent — ephemeral delivery

**`slack.ts:36` is the security-relevant line.** `ctx.thread.post` is a **public** thread post, harmless today only because the prompt carries no link. The moment it carries an authorize URL it is a credential in a public channel.

**Files:**
- Create: `packages/agent-workflows/mention/agent/lib/link.ts`, `tests/link.test.ts`
- Modify: `packages/agent-workflows/mention/agent/channels/slack.ts`

**Interfaces:**
- Consumes: `POST /internal/slack/link-intents` (Task 7)
- Produces: `signIntentRequest(secret, timestampSecs, body) -> { timestamp, signature }`, `requestAuthorizeUrl(principalId) -> Promise<string>`

**Run tooling from `packages/agent-workflows/mention/`** — this project is workspace-isolated; a root `npm install` inherits the root's bun overrides and fails.

- [ ] **Step 1: Write the failing test**

`packages/agent-workflows/mention/tests/link.test.ts`:

```ts
import { createHmac } from "node:crypto";
import { describe, expect, it } from "vitest";

import { signIntentRequest } from "../agent/lib/link.js";

describe("signIntentRequest", () => {
  it("signs HMAC-SHA256 over `{timestamp}.{body}` as lowercase hex", () => {
    const body = JSON.stringify({ slack_principal_id: "slack:T1:U1" });
    const { timestamp, signature } = signIntentRequest("s3cret", 1_700_000_000, body);

    expect(timestamp).toBe("1700000000");
    // The known-answer check: this MUST match temper_core::internal_sig::sign.
    const expected = createHmac("sha256", "s3cret")
      .update(`1700000000.${body}`)
      .digest("hex");
    expect(signature).toBe(expected);
    expect(signature).toMatch(/^[0-9a-f]{64}$/);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

```bash
cd packages/agent-workflows/mention && npm test
```
Expected: FAIL — cannot resolve `../agent/lib/link.js`.

- [ ] **Step 3: Implement**

`packages/agent-workflows/mention/agent/lib/link.ts`:

```ts
import { createHmac } from "node:crypto";

/**
 * The account-link intent call: agent -> temper-api.
 *
 * The signature covers THIS server-to-server call, not the URL the user clicks. temper's
 * `internal_sig` skew window is 30 seconds and a human clicks a Slack link minutes later,
 * so signing the user-facing URL would force that gate open. What the user gets is the
 * IdP's own authorize URL with an opaque, single-use, DB-backed state.
 *
 * Scheme (must match `temper_core::internal_sig::sign` byte for byte):
 *   HMAC-SHA256(secret, "{unix_timestamp}.{raw_body}") -> lowercase hex
 */
export function signIntentRequest(
  secret: string,
  timestampSecs: number,
  body: string,
): { timestamp: string; signature: string } {
  const timestamp = String(Math.floor(timestampSecs));
  const signature = createHmac("sha256", secret)
    .update(`${timestamp}.${body}`)
    .digest("hex");
  return { timestamp, signature };
}

/**
 * Ask temper for an authorize URL for this principal.
 *
 * `principalId` is passed WHOLE. It has 2-4 segments and must never be split.
 */
export async function requestAuthorizeUrl(principalId: string): Promise<string> {
  const baseUrl = requireEnv("TEMPER_API_URL");
  const secret = requireEnv("SLACK_LINK_SECRET");

  const body = JSON.stringify({ slack_principal_id: principalId });
  const { timestamp, signature } = signIntentRequest(secret, Date.now() / 1000, body);

  const res = await fetch(`${baseUrl.replace(/\/$/, "")}/internal/slack/link-intents`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Temper-Timestamp": timestamp,
      "X-Temper-Signature": signature,
    },
    body,
  });

  if (!res.ok) {
    throw new Error(`link-intents failed: ${res.status}`);
  }

  const json = (await res.json()) as { authorize_url: string };
  return json.authorize_url;
}

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}
```

- [ ] **Step 4: Run to verify it passes**

```bash
cd packages/agent-workflows/mention && npm test
```
Expected: PASS.

- [ ] **Step 5: Wire ephemeral delivery**

Rewrite `slack.ts:34-51`. Note `unlinkedPrompt` in `agent/lib/identity.ts` currently takes only the principal — extend it to take the URL, and update `tests/identity.test.ts` to match.

```ts
    // The link challenge is a CREDENTIAL: whoever opens it binds their temper identity to
    // this Slack principal. So it goes to the mentioning user ONLY — never `thread.post`,
    // which is public. The user id comes from `attributes.user_id`; NEVER from parsing
    // principalId, which has 2-4 segments.
    const userId = decision.auth.attributes.user_id;
    if (typeof userId !== "string") return null;

    try {
      const authorizeUrl = await requestAuthorizeUrl(decision.principalId);
      await ctx.thread.postEphemeral(userId, unlinkedPrompt(authorizeUrl));
    } catch (err) {
      // eve catches and logs a thrown error and drops the mention, so a failed intent
      // would be silent. Tell the user something honest instead of nothing.
      console.error("link intent failed", err);
      await ctx.thread.postEphemeral(
        userId,
        "I couldn't start the account-connect flow just now. Please try again in a moment.",
      );
    }

    // Deliberately DROP rather than dispatch (unchanged from T1). A turn under no identity
    // would run the model with no tools and nothing to ground an answer in, and the default
    // `message.completed` handler would post it. Until the link exists, the prompt IS the reply.
    return null;
```

> **Implementer note:** `decideIdentity` returns a union whose accepted arm must expose the `SessionAuthContext` for `attributes.user_id`. Read `agent/lib/identity.ts:44-46,78-82` and thread `auth` through the accepted arm if it is not already there, updating `tests/identity.test.ts`. Keep `identity.ts` pure — no fetch in it.

- [ ] **Step 6: Verify**

```bash
cd packages/agent-workflows/mention && npm test && npm run typecheck && npm run build
```
Expected: all pass. **`npm run build` is not optional** — eve resolves the agent by discovery at BUILD time, and typecheck+tests both stayed green while a missing file broke the deploy (that is why the build is a CI step now).

- [ ] **Step 7: Commit**

```bash
git add packages/agent-workflows/mention/agent packages/agent-workflows/mention/tests
git commit -m "feat(mention): ephemeral delivery of the account-link URL

slack.ts:36 was ctx.thread.post -- a PUBLIC thread post. Harmless while the
prompt carried no link; the moment it carries an authorize URL it is a
credential in a public channel. It becomes postEphemeral, with the user id from
attributes.user_id -- never from parsing principalId, which has 2-4 segments.

The HMAC signs the agent->API intent call, not the user-clicked URL: the skew
window is 30s and humans click minutes later."
```

---

## Task 9: End-to-end tests

`test-db` green is a false signal for access-semantics changes, and this is squarely one.

**Files:**
- Create: `tests/e2e/tests/slack_link_test.rs`

**Interfaces:**
- Consumes: the whole flow

- [ ] **Step 1: Write the tests**

Follow the existing harness in `tests/e2e/tests/common/` — read it first and match how a sibling test spawns the server and mints a JWT against the JWKS fixtures (`tests/e2e/tests/fixtures/`). The IdP token endpoint is external, so stub it with `wiremock` and point `SLACK_LINK_*` config at the stub.

The two load-bearing tests:

```rust
/// D3. Asserting the refusal alone would not catch a regression that creates the profile
/// and then errors, so assert the ABSENCE of the row.
#[tokio::test]
async fn callback_with_an_unknown_identity_creates_no_profile() {
    // ... harness setup; wiremock returns a token whose `sub` matches no kb_profile_auth_links row
    let before = count_profiles(&pool).await;

    let res = client.get(&callback_url_with_valid_state_and_code()).send().await.unwrap();

    assert_eq!(res.status(), 200, "the callback always renders a page");
    let body = res.text().await.unwrap();
    assert!(body.contains("No temper account is linked"));
    assert_eq!(count_profiles(&pool).await, before, "lookup-only must not mint a profile");
    assert_eq!(count_slack_links(&pool).await, 0);
}

/// D6. The single-use invariant, end to end.
#[tokio::test]
async fn a_replayed_state_is_rejected() {
    // ... harness setup with a profile that DOES exist
    let url = callback_url_with_valid_state_and_code();

    let first = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert!(first.contains("Linked as"));

    let second = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert!(second.contains("expired or was already used"));
    assert_eq!(count_slack_links(&pool).await, 1, "the replay must not write a second row");
}
```

Add a third, since it is the acceptance criterion and nearly free once the harness exists:

```rust
/// Re-linking the same Slack user is idempotent — no duplicate rows.
#[tokio::test]
async fn relinking_the_same_principal_is_idempotent() {
    // ... link once, then run a whole second intent+callback for the SAME principal and profile
    assert_eq!(count_slack_links(&pool).await, 1);
}
```

- [ ] **Step 2: Run**

```bash
cargo make test-e2e
```
Expected: PASS.

> **Implementer note:** the e2e suite spawns the `temper` binary and does **not** rebuild it. If a test spawns the CLI, `cargo build -p temper-cli` first. These tests drive HTTP directly, so that likely does not apply — but check the harness before blaming a failure on your code.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/slack_link_test.rs
git commit --no-verify -m "test(slack-link): the two load-bearing invariants, end to end

test-db green is a false signal for access-semantics changes.

The lookup-only test asserts the ABSENCE of a kb_profiles row, not just the
refusal: a regression that creates the profile and then errors would pass an
assertion on the message alone."
```

---

## Task 10: sqlx cache regeneration + full verification

Every prior task committed with `--no-verify` because new `query!()` macros fail offline until the cache exists. This task makes the tree honest.

**Files:**
- Modify: `.sqlx/`, `crates/temper-services/.sqlx/`, `crates/temper-api/.sqlx/`, `tests/e2e/.sqlx/`

- [ ] **Step 1: Regenerate, in this order**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
```

> Each `prepare` **rewrites its cache directory wholesale**, pruning entries no longer emitted — expect churn beyond your own queries and do not treat it as noise. `prepare-api` materializes ~207 **untracked** `.sqlx` files: **never `git add .sqlx` wholesale.** Stage the specific files git reports as modified/new for the queries this plan added.

- [ ] **Step 2: Verify offline**

```bash
cargo make check
```
Expected: clean. This is the honest probe — `cargo make` forces `SQLX_OFFLINE=true`, so a missing cache entry fails **here** and nowhere else (the CI clippy job compiles against a live DB and will not catch it).

- [ ] **Step 3: Full test sweep**

```bash
cargo make test           # unit, no DB
cargo make test-db        # integration
cargo make test-e2e       # end to end
cd packages/agent-workflows/mention && npm test && npm run typecheck && npm run build
```
Expected: pass. `test-all` shows one pre-existing streaming-embed timeout unrelated to this work — do not chase it.

- [ ] **Step 4: Commit**

```bash
git add .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx tests/e2e/.sqlx
git commit -m "chore(sqlx): regenerate offline caches for the slack-link queries"
```

- [ ] **Step 5: Merge main, then push and open the PR**

```bash
git fetch origin && git merge origin/main
cargo make check
git push -u origin jct/slack-account-link
```

Then open the PR. Its body must name the two operator prerequisites, because **the flow cannot work until they are done and neither is discoverable from the code**:

1. **Register the redirect_uri.** `https://<host>/api/auth/slack/callback` in Auth0's Allowed Callback URLs, or in `AS_CLIENTS` on an AS instance. `oauth/clients.ts:8` names the exact attack if this is skipped.
2. **Set the env:** `SLACK_LINK_CLIENT_ID`, `SLACK_LINK_SECRET` (temper-api **and** the `temper-mention` project), `PUBLIC_BASE_URL`, `TEMPER_API_URL` (on temper-mention).

Also state the one accepted unknown: **the AS-mode derivation is verified by trial on a real AS instance after merge.** No local test reproduces a real AS env, so a green suite evidences nothing either way. If it fails, the failure is legible — a wrong `authorize_url`/`token_url` base — and contained to `link_provider::derive`.

---

## Self-Review

**Spec coverage:**

| Spec | Task |
|---|---|
| D1 PKCE grant, T3 seam | 7 (`run_callback`'s seam comment) |
| D2 Rust home, client half | 5, 7 |
| D3 lookup-only | **4**, 9 |
| D4 rebind is a feature | 3 (`upsert_slack_link`) |
| D5 HMAC gates agent→API | 6, 8 |
| D6 opaque DB-backed state | 2, 3, 9 |
| D7 browser confirm, no Slack credential | 7 |
| D8 temper-auth, scoped | 1 |
| `kb_slack_link_intents` | 2 |
| Agent `postEphemeral` | 8 |
| Error handling (HTML, non-leaking) | 7 |
| Testing (both invariants, e2e) | 9 |
| Ops (redirect_uri, env) | 10 Step 5 |
| Decomposition (2 PRs) | 1 = PR 1; 2–10 = PR 2 |

**Gap found and closed:** the spec's Task-4 narrowing had to preserve `authenticate`'s post-resolution gates (active, system access). A lookup-only path that skipped them would be a hole rather than a narrowing — called out explicitly in Task 4's implementer note.

**Type consistency:** `generate_pkce_pair`, `AuthorizeParams`, `build_authorize_url`, `TokenResponse` (Task 1) are consumed with those exact names in Tasks 5 and 7. `ConsumedIntent { code_verifier, slack_principal_id }` (Task 3) is destructured identically in Task 7. `SlackLinkConfig { client_id, hmac_secret, public_base_url }` (Task 5) is read with those field names in Tasks 6 and 7. `signIntentRequest` (Task 8) matches `internal_sig::sign`'s `"{timestamp}.{body}"` scheme, pinned by a known-answer test.

**Placeholders:** none. Three implementer notes ask the engineer to *read named code and follow its structure* (`authenticate`'s gate tail, `routes.rs`'s layering, the e2e harness) rather than guess — each names the exact file, the lines, and the invariant that must hold.
