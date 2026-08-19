# Machine-principal Phase B1 (issuer grant) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let temper *issue* `client_credentials` machine credentials from its own OAuth Authorization Server, so an OpenAPI-derived client (temper-rb first) can obtain an M2M token with no Auth0 application, and a self-hosted instance can mint one at all.

**Architecture:** Rust *issues* the credential (extends Phase A's CLI + registration service; `sha2`/`rand` already present) and TypeScript *verifies* it (a third grant on the AS). The secret is a temper-generated 32-byte value stored as its SHA-256 hex — `format!("{:x}", Sha256)` in Rust is byte-identical to `createHash("sha256").digest("hex")` in TS, so nothing is duplicated across languages. A temper-issued token is just a JWT carrying `gty:"client-credentials"` + `azp:<client_id>`, validated under the JWKS the API already trusts, so `normalize_machine` and Phase A's `resolve_machine_from_claims` gate are **untouched**.

**Tech Stack:** Rust (temper-services, temper-core, temper-api, temper-client, temper-cli), sqlx + PostgreSQL, TypeScript (temper-cloud OAuth AS: `jose`, `@neondatabase/serverless`), vitest, cargo-nextest.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-10-machine-principal-phase-b1-issuer-grant-design.md`. Decisions D1–D9.
- **SHA-256, never argon2** (D1). Reuse `hashToken()` on the TS side; `format!("{:x}", Sha256::new()...finalize())` on the Rust side (lowercase hex, matches TS). No new crypto dependency.
- **The verifier does not change** (D4). Do not touch `normalize_machine` (`temper-services/src/auth/normalize.rs`) or `resolve_machine_from_claims`. No JWKS change.
- **`issuer='temper'` marks the credential; `auth_provider` stays `'auth0-m2m'`** (D5) — the machine-principal namespace `normalize_machine` emits for all machine tokens. Reuse `crate::auth::MACHINE_PROVIDER_TAG`.
- **No plaintext secret is ever stored** — only its SHA-256 hex. The plaintext is returned once, at issuance/rotation.
- **`is_system_admin` is load-bearing** (D9). Every new handler calls `require_admin` before any mutation (auth before writes).
- **Migration:** `migrations/20260711000070_machine_client_secrets.sql` — additive, all-nullable columns; applies on PG 17 (Neon) and PG 18 (local). Never edit a shipped migration.
- **Typed structs over inline JSON.** All new request/response bodies are structs in `temper-core::types::machine`. Never `serde_json::json!()`.
- **Persistence layer owns SQL.** New `sqlx::query!()` lives in `temper-services/src/services/`, never in a handler or CLI action.
- **sqlx offline cache:** after adding SQL, apply the migration to the dev DB and regenerate caches (Task 11). Local dev/CI use `SQLX_OFFLINE=true`; `cargo make check` is the honest offline probe.
- **DB tests need `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`** and Docker Postgres (`cargo make docker-up`). `#[sqlx::test]` tests apply migrations to an ephemeral per-test DB via the migrator; compile-time macro checks still need the live schema or the cache.
- **Rust dep additions:** `temper-services/Cargo.toml` gains `sha2 = "0.10"` and `base64 = "0.22"` (`rand = "0.8"` already present).

---

### Task 1: Migration — secret columns on `kb_machine_clients`

**Files:**
- Create: `migrations/20260711000070_machine_client_secrets.sql`

**Interfaces:**
- Produces: four nullable columns `secret_hash`, `secret_hash_previous`, `secret_previous_expires_at`, `secret_rotated_at` on `kb_machine_clients`.

- [ ] **Step 1: Write the migration**

Create `migrations/20260711000070_machine_client_secrets.sql`:

```sql
-- Machine-principal Phase B1: temper as a client_credentials issuer.
-- Spec 2026-07-10-machine-principal-phase-b1-issuer-grant-design.md (D1, D5, D6).
--
-- Additive and all-nullable: auth0-m2m rows are untouched (secret_hash NULL; they keep
-- verifying against Auth0's JWKS). issuer='temper' rows carry a SHA-256 hex of a
-- temper-minted secret. Two verification paths, keyed on `issuer`. No plaintext is ever stored.
ALTER TABLE kb_machine_clients
  ADD COLUMN secret_hash                TEXT        NULL,
  ADD COLUMN secret_hash_previous       TEXT        NULL,
  ADD COLUMN secret_previous_expires_at TIMESTAMPTZ NULL,
  ADD COLUMN secret_rotated_at          TIMESTAMPTZ NULL;

COMMENT ON COLUMN kb_machine_clients.secret_hash IS
  'SHA-256 hex of the current client secret, for issuer=temper rows only. NULL for auth0-m2m rows, which verify against Auth0 JWKS. No plaintext is ever stored (D1).';
COMMENT ON COLUMN kb_machine_clients.secret_hash_previous IS
  'The second live secret during rotation; accepted only until secret_previous_expires_at. Zero-downtime secret rotation, capped at two live secrets (D6).';
COMMENT ON COLUMN kb_machine_clients.secret_previous_expires_at IS
  'Expiry of secret_hash_previous. Past this instant, only secret_hash is accepted.';
COMMENT ON COLUMN kb_machine_clients.secret_rotated_at IS
  'Audit stamp of the last secret rotation.';
```

- [ ] **Step 2: Apply it to the dev database**

Run: `cargo make docker-up` (if not already running), then
`DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run --source migrations`
Expected: `Applied 20260711000070/migrate machine client secrets`.

- [ ] **Step 3: Verify the columns exist**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development psql "$DATABASE_URL" -c "\d kb_machine_clients" | grep secret_`
Expected: four `secret_*` rows printed.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260711000070_machine_client_secrets.sql
git commit -m "feat(migration): add secret columns to kb_machine_clients (Phase B1)"
```

---

### Task 2: Rust secret module — mint + hash

**Files:**
- Create: `crates/temper-services/src/auth/secret.rs`
- Modify: `crates/temper-services/src/auth/mod.rs` (add `pub mod secret;`)
- Modify: `crates/temper-services/Cargo.toml` (add `sha2`, `base64`)

**Interfaces:**
- Produces:
  - `sha256_hex(input: &str) -> String` — lowercase hex, identical to TS `hashToken`.
  - `struct MintedSecret { pub plaintext: String, pub hash: String }`
  - `mint_secret() -> MintedSecret` — 32 random bytes, base64url-no-pad plaintext, SHA-256 hex hash.
  - `mint_client_id() -> String` — `tmpr_<base64url of 16 random bytes>`.

- [ ] **Step 1: Add dependencies**

In `crates/temper-services/Cargo.toml`, under `[dependencies]`, add alongside the existing `rand = "0.8"`:

```toml
sha2 = "0.10"
base64 = "0.22"
```

- [ ] **Step 2: Write the failing test**

Create `crates/temper-services/src/auth/secret.rs`:

```rust
//! Temper-minted machine credentials (Phase B1, D1/D3). temper generates the client_id and
//! the secret; only the secret's SHA-256 hex is ever stored. `sha256_hex` is byte-identical
//! to the TS AS's `hashToken` (`createHash("sha256").digest("hex")`), so a hash written here
//! verifies against a secret presented at the TS `/oauth/token` endpoint.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore as _;
use sha2::{Digest, Sha256};

/// Prefix on temper-minted client ids, distinguishing them from Auth0 client ids at a glance.
const CLIENT_ID_PREFIX: &str = "tmpr_";

/// Lowercase SHA-256 hex of `input`. Matches the TS AS's `hashToken`.
pub fn sha256_hex(input: &str) -> String {
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    format!("{:x}", h.finalize())
}

/// A freshly minted secret: the plaintext (returned once, never stored) and its stored hash.
#[derive(Debug)]
pub struct MintedSecret {
    pub plaintext: String,
    pub hash: String,
}

/// Mint a 32-byte random secret (base64url-no-pad) and its SHA-256 hex.
pub fn mint_secret() -> MintedSecret {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let plaintext = URL_SAFE_NO_PAD.encode(bytes);
    let hash = sha256_hex(&plaintext);
    MintedSecret { plaintext, hash }
}

/// Mint a temper client id: `tmpr_` + base64url of 16 random bytes.
pub fn mint_client_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("{CLIENT_ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_answer() {
        // echo -n "abc" | sha256sum
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn mint_secret_hash_is_the_sha256_of_its_plaintext() {
        let s = mint_secret();
        assert_eq!(s.hash, sha256_hex(&s.plaintext));
        assert_eq!(s.hash.len(), 64, "sha256 hex is 64 chars");
        assert!(s.plaintext.len() >= 43, "32 bytes base64url-no-pad is 43 chars");
    }

    #[test]
    fn mint_client_id_is_prefixed_and_unique() {
        let a = mint_client_id();
        let b = mint_client_id();
        assert!(a.starts_with("tmpr_"));
        assert_ne!(a, b, "two mints must differ");
    }
}
```

- [ ] **Step 3: Wire the module**

In `crates/temper-services/src/auth/mod.rs`, after line 19 (`mod normalize;`), add:

```rust
pub mod secret;
```

- [ ] **Step 4: Run the tests**

Run: `cargo nextest run -p temper-services secret::tests`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/auth/secret.rs crates/temper-services/src/auth/mod.rs crates/temper-services/Cargo.toml Cargo.lock
git commit -m "feat(services): temper machine-secret minting + sha256_hex (Phase B1)"
```

---

### Task 3: Shared types + `apply_reach` refactor

**Files:**
- Modify: `crates/temper-core/src/types/machine.rs` (add three request/response types)
- Modify: `crates/temper-services/src/services/machine_registration_service.rs:57-101` (refactor `apply_reach` signature)

**Interfaces:**
- Produces (in `temper_core::types::machine`):
  - `struct IssueMachineRequest { label: String, owner_team_id: Option<Uuid>, teams: Vec<TeamSpec>, grants: Vec<GrantSpec> }`
  - `struct IssuedMachineCredential { client: MachineClient, client_secret: String }` — returned by both `issue` and `rotate_secret`; carries the one-time plaintext.
  - `struct RotateSecretRequest { grace_seconds: i64 }`
- Produces: `apply_reach(conn, caller, profile_id, teams: &[TeamSpec], grants: &[GrantSpec])` — now takes slices, so both `provision` and `issue` reuse it.

- [ ] **Step 1: Add the types**

In `crates/temper-core/src/types/machine.rs`, append after `RebindMachineRequest` (line 63):

```rust
/// Issue a temper-minted machine credential (Phase B1). temper mints the `client_id` AND a
/// secret (`issuer='temper'`), so — unlike `ProvisionMachineRequest` — there is no external
/// client id. Reach is plural and always explicit (D10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueMachineRequest {
    pub label: String,
    /// Recorded as `team_id`. Owner, not reach.
    pub owner_team_id: Option<Uuid>,
    pub teams: Vec<TeamSpec>,
    pub grants: Vec<GrantSpec>,
}

/// A one-time machine credential returned by `issue` and `rotate-secret`. The plaintext
/// `client_secret` is returned ONCE and never stored; only its SHA-256 hex persists (D1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuedMachineCredential {
    pub client: MachineClient,
    pub client_secret: String,
}

/// Rotate a temper-issued secret, leaving the previous secret valid for a grace window (D6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotateSecretRequest {
    /// Seconds the previous secret stays valid after rotation. Defaults at the CLI.
    pub grace_seconds: i64,
}
```

- [ ] **Step 2: Refactor `apply_reach` to take slices**

In `crates/temper-services/src/services/machine_registration_service.rs`, change the `apply_reach` signature (lines 57-62) from taking `req: &ProvisionMachineRequest` to taking slices, and update the two loops:

```rust
async fn apply_reach(
    conn: &mut sqlx::PgConnection,
    caller: ProfileId,
    profile_id: Uuid,
    teams: &[temper_core::types::machine::TeamSpec],
    grants: &[temper_core::types::machine::GrantSpec],
) -> ApiResult<()> {
    for team in teams {
```

and change `for grant in &req.grants {` to `for grant in grants {`.

- [ ] **Step 3: Update `provision`'s call site**

In the same file, in `provision` (line 168), change:

```rust
    apply_reach(&mut tx, caller, profile_id, &req.teams, &req.grants).await?;
```

- [ ] **Step 4: Compile-check**

Run: `cargo check -p temper-core -p temper-services`
Expected: compiles (existing `provision` tests still reference the same public API).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/machine.rs crates/temper-services/src/services/machine_registration_service.rs
git commit -m "feat(core): Phase B1 issue/rotate types; apply_reach takes slices"
```

---

### Task 4: `issue` service function

**Files:**
- Modify: `crates/temper-services/src/services/machine_registration_service.rs` (add `issue`, and a test)

**Interfaces:**
- Consumes: `crate::auth::secret::{mint_client_id, mint_secret}`, `IssueMachineRequest`, `IssuedMachineCredential`, the existing `create_agent_profile_and_link`, `provision_profile_entities`, `enroll_in_gating_team`, `apply_reach`, `machine_client_service::get`.
- Produces: `pub async fn issue(pool: &PgPool, caller: ProfileId, req: &IssueMachineRequest) -> ApiResult<IssuedMachineCredential>`.

- [ ] **Step 1: Write the failing test**

In `crates/temper-services/src/services/machine_registration_service.rs`, inside the existing `#[cfg(all(test, feature = "test-db"))] mod tests`, add (and add `IssueMachineRequest` to the `use temper_core::types::machine::{...}` import at the top of the test module):

```rust
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn issue_mints_a_temper_credential_with_a_stored_hash(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let cred = svc::issue(
            &pool,
            admin,
            &IssueMachineRequest {
                label: "sidekiq".to_string(),
                owner_team_id: None,
                teams: vec![],
                grants: vec![],
            },
        )
        .await
        .expect("issue");

        assert!(cred.client.client_id.starts_with("tmpr_"), "temper mints the id");
        assert_eq!(cred.client.issuer, "temper");
        assert!(!cred.client_secret.is_empty(), "plaintext returned once");
        assert_eq!(cred.client.registered_by_profile_id, *admin);

        // The stored hash is the SHA-256 of the returned plaintext; the plaintext itself is
        // never persisted.
        let stored: Option<String> = sqlx::query_scalar!(
            "SELECT secret_hash FROM kb_machine_clients WHERE id = $1",
            cred.client.id,
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            stored.as_deref(),
            Some(crate::auth::secret::sha256_hex(&cred.client_secret).as_str()),
        );

        // The auth link uses the machine-principal namespace, NOT 'temper' (D5).
        let link = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_profile_auth_links \
              WHERE auth_provider = 'auth0-m2m' AND auth_provider_user_id = $1",
            cred.client.client_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count link");
        assert_eq!(link, Some(1));
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db issue_mints`
Expected: FAIL — `svc::issue` does not exist.

- [ ] **Step 3: Implement `issue`**

In the same file, after `provision` (line 190), add:

```rust
/// Issue a temper-minted machine credential (Phase B1). temper generates the `client_id` and
/// the secret; the SHA-256 hex of the secret is stored, the plaintext is returned once. Creates
/// the agent profile, auth link, emitters, gating-team membership, and reach — all in one
/// transaction, exactly like `provision`, but with `issuer='temper'` and a `secret_hash`.
pub async fn issue(
    pool: &PgPool,
    caller: ProfileId,
    req: &temper_core::types::machine::IssueMachineRequest,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    let client_id = crate::auth::secret::mint_client_id();
    let secret = crate::auth::secret::mint_secret();

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    let (profile_id, handle) =
        profile_service::create_agent_profile_and_link(&mut tx, &client_id)
            .await
            .map_err(|e| map_duplicate_from_conflict(e, &client_id))?;

    profile_service::provision_profile_entities(&mut tx, profile_id, &handle).await?;
    enroll_in_gating_team(&mut tx, profile_id).await?;
    apply_reach(&mut tx, caller, profile_id, &req.teams, &req.grants).await?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_machine_clients
               (client_id, issuer, label, profile_id, team_id, registered_by_profile_id, secret_hash)
           VALUES ($1, 'temper', $2, $3, $4, $5, $6)
           RETURNING id"#,
        client_id,
        req.label,
        profile_id,
        req.owner_team_id,
        *caller,
        secret.hash,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &client_id))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    let client = machine_client_service::get(pool, id).await?;
    Ok(temper_core::types::machine::IssuedMachineCredential {
        client,
        client_secret: secret.plaintext,
    })
}
```

Add this small helper next to `map_duplicate` (it dedupes the identical `match` block `provision` already uses inline at lines 157-164):

```rust
/// The auth-link unique constraint fires before the registration row's; turn its Conflict
/// into a client-id-naming message.
fn map_duplicate_from_conflict(err: ApiError, client_id: &str) -> ApiError {
    match err {
        ApiError::Conflict(_) => {
            ApiError::Conflict(format!("machine client '{client_id}' is already registered"))
        }
        other => other,
    }
}
```

(Optionally refactor `provision`'s inline `match` at lines 157-164 to call `map_duplicate_from_conflict` too — a pure DRY cleanup, no behavior change.)

- [ ] **Step 4: Run the test**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db issue_mints`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/services/machine_registration_service.rs
git commit -m "feat(services): issue temper-minted machine credentials (Phase B1)"
```

---

### Task 5: `rotate_secret` service function

**Files:**
- Modify: `crates/temper-services/src/services/machine_client_service.rs` (add `rotate_secret`, and tests)

**Interfaces:**
- Consumes: `crate::auth::secret::mint_secret`, `IssuedMachineCredential`, `get`.
- Produces: `pub async fn rotate_secret(pool: &PgPool, id: Uuid, grace_seconds: i64) -> ApiResult<IssuedMachineCredential>`.

- [ ] **Step 1: Write the failing tests**

In `crates/temper-services/src/services/machine_client_service.rs`, inside the existing `#[cfg(all(test, feature = "test-db"))] mod tests`, add a helper to seed a temper-issued row and two tests:

```rust
    /// Seed a temper-issued client with a known secret hash. Returns (machine_client id, plaintext).
    async fn seed_temper_issued(pool: &PgPool, client_id: &str, secret: &str) -> Uuid {
        let profile_id = seed_agent_link(pool, client_id).await;
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_machine_clients \
               (id, client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
             VALUES ($1, $2, 'temper', 'test', $3, $3, $4)",
            id,
            client_id,
            profile_id,
            crate::auth::secret::sha256_hex(secret),
        )
        .execute(pool)
        .await
        .expect("seed temper-issued");
        id
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rotate_secret_moves_current_to_previous_with_expiry(pool: PgPool) {
        let id = seed_temper_issued(&pool, "tmpr_rot", "old-secret").await;

        let cred = svc::rotate_secret(&pool, id, 3600).await.expect("rotate");

        // A fresh plaintext is returned and its hash is the new current.
        let row = sqlx::query!(
            "SELECT secret_hash, secret_hash_previous, secret_previous_expires_at, secret_rotated_at \
               FROM kb_machine_clients WHERE id = $1",
            id,
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(
            row.secret_hash.as_deref(),
            Some(crate::auth::secret::sha256_hex(&cred.client_secret).as_str()),
            "current is the new secret"
        );
        assert_eq!(
            row.secret_hash_previous.as_deref(),
            Some(crate::auth::secret::sha256_hex("old-secret").as_str()),
            "previous is the old secret"
        );
        assert!(row.secret_previous_expires_at.is_some(), "previous has a grace expiry");
        assert!(row.secret_rotated_at.is_some(), "rotation is stamped");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn rotate_secret_rejects_a_non_temper_issued_client(pool: PgPool) {
        // A plain auth0-m2m registration (issuer default), no secret.
        let id = seed_registered(&pool, "auth0-client").await;

        let err = svc::rotate_secret(&pool, id, 3600)
            .await
            .expect_err("must reject");
        assert!(
            matches!(err, crate::error::ApiError::BadRequest(_)),
            "auth0-m2m secrets are managed by the IdP, not temper; got {err:?}"
        );
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db rotate_secret`
Expected: FAIL — `svc::rotate_secret` does not exist.

- [ ] **Step 3: Implement `rotate_secret`**

In the same file, after `revoke` (line 97), add:

```rust
/// Rotate a temper-issued secret (Phase B1, D6). Moves the current secret to `previous` with a
/// grace window, installs a fresh current, and returns the new plaintext once. Rejects a client
/// that temper did not issue (its secret lives at its IdP) or one already revoked.
pub async fn rotate_secret(
    pool: &PgPool,
    id: Uuid,
    grace_seconds: i64,
) -> ApiResult<temper_core::types::machine::IssuedMachineCredential> {
    let existing = get(pool, id).await?;
    if existing.issuer != "temper" {
        return Err(ApiError::BadRequest(format!(
            "machine client '{}' was not issued by temper (issuer '{}'); its secret is managed by its IdP",
            existing.client_id, existing.issuer
        )));
    }
    if existing.revoked_at.is_some() {
        return Err(ApiError::BadRequest(format!(
            "machine client '{}' is revoked; issue a new credential instead",
            existing.client_id
        )));
    }

    let secret = crate::auth::secret::mint_secret();
    sqlx::query!(
        r#"UPDATE kb_machine_clients
              SET secret_hash_previous       = secret_hash,
                  secret_previous_expires_at = now() + make_interval(secs => $2),
                  secret_hash                = $3,
                  secret_rotated_at          = now()
            WHERE id = $1"#,
        id,
        grace_seconds as f64,
        secret.hash,
    )
    .execute(pool)
    .await?;

    let client = get(pool, id).await?;
    Ok(temper_core::types::machine::IssuedMachineCredential {
        client,
        client_secret: secret.plaintext,
    })
}
```

- [ ] **Step 4: Run the tests**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db rotate_secret`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-services/src/services/machine_client_service.rs
git commit -m "feat(services): rotate temper-issued secret with grace window (Phase B1)"
```

---

### Task 6: API handlers + routes + OpenAPI-routes allowlist

**Files:**
- Modify: `crates/temper-api/src/handlers/machine_clients.rs` (add `issue`, `rotate_secret` handlers)
- Modify: `crates/temper-api/src/routes.rs:164-173` (mount two routes)
- Modify: `.github/scripts/check-openapi-routes.sh` (allowlist the two new operator-only routes)

**Interfaces:**
- Consumes: `machine_registration_service::issue`, `machine_client_service::rotate_secret`, `IssueMachineRequest`, `RotateSecretRequest`, `IssuedMachineCredential`.
- Produces: `POST /api/machine-clients/issue`, `POST /api/machine-clients/{id}/rotate-secret`.

- [ ] **Step 1: Add the handlers**

In `crates/temper-api/src/handlers/machine_clients.rs`, extend the `use temper_core::types::machine::{...}` import (line 15) to include `IssueMachineRequest, IssuedMachineCredential, RotateSecretRequest`, then add after `revoke` (line 93):

```rust
pub async fn issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<IssueMachineRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let caller = require_admin(&state, &auth.0).await?;
    let cred = machine_registration_service::issue(&state.pool, caller, &body).await?;
    Ok(Json(cred))
}

pub async fn rotate_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RotateSecretRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    require_admin(&state, &auth.0).await?;
    let cred = machine_client_service::rotate_secret(&state.pool, id, body.grace_seconds).await?;
    Ok(Json(cred))
}
```

- [ ] **Step 2: Mount the routes**

In `crates/temper-api/src/routes.rs`, in the same builder block that mounts `/api/machine-clients` (around lines 164-173), add two routes. Put the static `/issue` route and the `/{id}/rotate-secret` route alongside the others:

```rust
        .route(
            "/api/machine-clients/issue",
            post(handlers::machine_clients::issue),
        )
        .route(
            "/api/machine-clients/{id}/rotate-secret",
            post(handlers::machine_clients::rotate_secret),
        )
```

(These are plain `.route()` mounts — out of the OpenAPI contract, exactly like the existing machine-client routes. `POST /issue` cannot be shadowed by `GET/DELETE /{id}`: different methods, and matchit prioritizes the static segment regardless.)

- [ ] **Step 3: Allowlist the routes in the OpenAPI-routes check**

In `.github/scripts/check-openapi-routes.sh`, find where the existing machine-client routes are allowlisted (grep for `machine-clients`) and add the two new paths to the same operator-only allowlist array, following the exact syntax already used for `/api/machine-clients/{id}/rebind`.

- [ ] **Step 4: Compile-check**

Run: `cargo check -p temper-api`
Expected: compiles.

- [ ] **Step 5: Run the OpenAPI-routes check**

Run: `bash .github/scripts/check-openapi-routes.sh`
Expected: passes (the new routes are recognized as intentionally-out-of-contract).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/handlers/machine_clients.rs crates/temper-api/src/routes.rs .github/scripts/check-openapi-routes.sh
git commit -m "feat(api): POST issue + rotate-secret machine-client routes (Phase B1)"
```

---

### Task 7: temper-client methods + CLI (`issue`, `rotate-secret`)

**Files:**
- Modify: `crates/temper-client/src/machine.rs` (add `issue`, `rotate_secret` methods)
- Modify: `crates/temper-cli/src/commands/admin_machine.rs` (add `issue_remote`, `rotate_secret_remote`; extract a shared reach resolver)
- Modify: `crates/temper-cli/src/cli.rs:919-966` (add `Issue`, `RotateSecret` variants)
- Modify: `crates/temper-cli/src/main.rs:696-771` (dispatch the two variants)

**Interfaces:**
- Consumes: `IssueMachineRequest`, `RotateSecretRequest`, `IssuedMachineCredential`.
- Produces: `client.machine_clients().issue(&req)`, `.rotate_secret(id, &req)`; CLI `temper admin machine issue`, `temper admin machine rotate-secret`.

- [ ] **Step 1: Add the client methods**

In `crates/temper-client/src/machine.rs`, extend the `temper_core::types::machine` import to include `IssueMachineRequest, IssuedMachineCredential, RotateSecretRequest`, then add to the `impl MachineClientsClient` block:

```rust
    /// Issue a temper-minted machine credential. Returns the one-time plaintext secret.
    pub async fn issue(&self, body: &IssueMachineRequest) -> Result<IssuedMachineCredential> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/machine-clients/issue").json(body);
        self.http
            .send_json(&Method::POST, "/api/machine-clients/issue", req, Some(&token))
            .await
    }

    /// Rotate a temper-issued secret, leaving the previous valid for a grace window.
    pub async fn rotate_secret(
        &self,
        id: Uuid,
        body: &RotateSecretRequest,
    ) -> Result<IssuedMachineCredential> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/machine-clients/{id}/rotate-secret");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }
```

- [ ] **Step 2: Extract a shared reach resolver in the CLI action module**

In `crates/temper-cli/src/commands/admin_machine.rs`, extend the type import to include `IssueMachineRequest, IssuedMachineCredential, RotateSecretRequest`, and extract the owner-team/team/cogmap resolution that `provision_remote` does inline (lines 37-74) into a helper both callers use:

```rust
/// Resolve `--owner-team`, repeatable `--team`, and repeatable `--cogmap` refs into ids/specs.
/// Reach is plural and never inferred from `--owner-team` (D6/D10).
async fn resolve_reach(
    client: &temper_client::TemperClient,
    owner_team: Option<&str>,
    teams: &[String],
    cogmaps: &[String],
) -> Result<(Option<uuid::Uuid>, Vec<TeamSpec>, Vec<GrantSpec>)> {
    let owner_team_id = match owner_team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let mut team_specs = Vec::with_capacity(teams.len());
    for raw in teams {
        let (team_ref, role) = split_spec(raw);
        team_specs.push(TeamSpec {
            team_id: crate::actions::cogmap::resolve_team_id(client, &team_ref).await?,
            role: role.unwrap_or_else(|| "member".to_string()),
        });
    }

    let mut grant_specs = Vec::with_capacity(cogmaps.len());
    for raw in cogmaps {
        let (cogmap_ref, mode) = split_spec(raw);
        let cogmap_id = temper_workflow::operations::parse_ref(&cogmap_ref)
            .map_err(|e| TemperError::Api(format!("invalid cogmap ref '{cogmap_ref}': {e}")))?
            .0;
        let can_write = match mode.as_deref() {
            None | Some("rw") => true,
            Some("ro") => false,
            Some(other) => {
                return Err(TemperError::Api(format!(
                    "invalid --cogmap mode ':{other}' for '{cogmap_ref}' (expected 'ro' or 'rw')"
                )))
            }
        };
        grant_specs.push(GrantSpec { cogmap_id, can_write });
    }

    Ok((owner_team_id, team_specs, grant_specs))
}
```

Then rewrite `provision_remote`'s body (lines 37-82) to use it:

```rust
    let (owner_team_id, team_specs, grant_specs) =
        resolve_reach(client, owner_team, teams, cogmaps).await?;

    let req = ProvisionMachineRequest {
        client_id: client_id.to_string(),
        label: label.to_string(),
        owner_team_id,
        teams: team_specs,
        grants: grant_specs,
    };
    let row = client
        .machine_clients()
        .provision(&req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&row, fmt)?);
    Ok(())
```

- [ ] **Step 3: Add the two new actions**

In the same file, add:

```rust
/// Issue a temper-minted machine credential. Prints the one-time secret.
pub async fn issue_remote(
    client: &temper_client::TemperClient,
    label: &str,
    owner_team: Option<&str>,
    teams: &[String],
    cogmaps: &[String],
    fmt: OutputFormat,
) -> Result<()> {
    let (owner_team_id, team_specs, grant_specs) =
        resolve_reach(client, owner_team, teams, cogmaps).await?;

    let req = IssueMachineRequest {
        label: label.to_string(),
        owner_team_id,
        teams: team_specs,
        grants: grant_specs,
    };
    let cred: IssuedMachineCredential = client
        .machine_clients()
        .issue(&req)
        .await
        .map_err(crate::commands::client_err)?;

    // Render the whole credential (JSON by default) so an agent capturing stdout gets the
    // secret; warn on stderr for a human at a TTY.
    println!("{}", crate::format::render(&cred, fmt)?);
    crate::output::warning(
        "client_secret is shown ONCE and never stored — capture it now; it cannot be retrieved later.",
    );
    Ok(())
}

/// Rotate a temper-issued secret. Prints the new one-time secret.
pub async fn rotate_secret_remote(
    client: &temper_client::TemperClient,
    id: &str,
    grace_seconds: i64,
    fmt: OutputFormat,
) -> Result<()> {
    let machine_id = parse_uuid("machine client id", id)?;
    let req = RotateSecretRequest { grace_seconds };
    let cred: IssuedMachineCredential = client
        .machine_clients()
        .rotate_secret(machine_id, &req)
        .await
        .map_err(crate::commands::client_err)?;

    println!("{}", crate::format::render(&cred, fmt)?);
    crate::output::warning(
        "New client_secret is shown ONCE. The previous secret stays valid until the grace window expires.",
    );
    Ok(())
}
```

- [ ] **Step 4: Add the clap variants**

In `crates/temper-cli/src/cli.rs`, inside `enum AdminMachineAction` (after the `Provision` variant, ~line 943), add:

```rust
    /// Issue a temper-minted machine credential (client_credentials on temper's own AS).
    /// temper mints the client id and a secret; the secret is printed once.
    Issue {
        /// Human-facing label
        #[arg(long)]
        label: String,
        /// Team recorded as this machine's OWNER. Not its reach.
        #[arg(long = "owner-team")]
        owner_team: Option<String>,
        /// Team to enroll in, as `<ref>` or `<ref>:<role>` (role defaults to `member`). Repeatable.
        #[arg(long = "team")]
        teams: Vec<String>,
        /// Cogmap to grant, as `<ref>` or `<ref>:ro` (defaults to read+write). Repeatable.
        #[arg(long = "cogmap")]
        cogmaps: Vec<String>,
    },
    /// Rotate a temper-issued secret. The previous secret stays valid for a grace window.
    RotateSecret {
        /// The machine client to rotate (its `id`, from `list`)
        id: String,
        /// Seconds the previous secret stays valid after rotation (default 86400 = 24h).
        #[arg(long = "grace", default_value_t = 86_400)]
        grace_seconds: i64,
    },
```

- [ ] **Step 5: Dispatch the variants**

In `crates/temper-cli/src/main.rs`, inside the `AdminAction::Machine { action } => match action { ... }` block (after the `Provision` arm, ~line 716), add — mirroring the `with_client(...)` wrapping the sibling arms use:

```rust
        AdminMachineAction::Issue {
            label,
            owner_team,
            teams,
            cogmaps,
        } => {
            temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    commands::admin_machine::issue_remote(
                        client,
                        &label,
                        owner_team.as_deref(),
                        &teams,
                        &cogmaps,
                        output_format,
                    )
                    .await
                })
            })
            .await
        }
        AdminMachineAction::RotateSecret { id, grace_seconds } => {
            temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    commands::admin_machine::rotate_secret_remote(
                        client,
                        &id,
                        grace_seconds,
                        output_format,
                    )
                    .await
                })
            })
            .await
        }
```

(Match the exact `with_client`/`output_format` wrapping of the neighbouring `Provision`/`Rebind` arms; copy their shape verbatim if the above differs.)

- [ ] **Step 6: Build the CLI binary and its libraries**

Run: `cargo build -p temper-cli -p temper-client`
Expected: compiles.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-client/src/machine.rs crates/temper-cli/src/commands/admin_machine.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "feat(cli): temper admin machine issue + rotate-secret (Phase B1)"
```

---

### Task 8: TS — machine-token minting + secret verification module

**Files:**
- Modify: `packages/temper-cloud/src/oauth/mint.ts` (add `mintMachineAccessToken`)
- Create: `packages/temper-cloud/src/oauth/machine-clients.ts` (verify + coarse touch)

**Interfaces:**
- Produces:
  - `mintMachineAccessToken(clientId: string): Promise<string>` — EdDSA JWT with `sub:"<id>@clients"`, `azp:"<id>"`, `gty:"client-credentials"`, no email.
  - `verifyMachineSecret(db: NeonClient, clientId: string, clientSecret: string): Promise<boolean>`
  - `touchMachineLastSeen(db: NeonClient, clientId: string): Promise<void>`

- [ ] **Step 1: Add `mintMachineAccessToken`**

In `packages/temper-cloud/src/oauth/mint.ts`, append after `mintAccessToken` (line 52):

```ts
/**
 * Mints an EdDSA access token for a temper-issued machine principal. The claim shape mirrors an
 * Auth0 client_credentials token exactly — `gty:"client-credentials"`, `azp:<client_id>`,
 * `sub:"<client_id>@clients"`, no email — so `normalize_machine` (Rust) detects it unchanged.
 */
export async function mintMachineAccessToken(clientId: string): Promise<string> {
  const { key, kid } = await getSigningKey();
  const issuer = requireEnv("AS_ISSUER");
  const audience = requireEnv("AS_AUDIENCE");
  const nowSeconds = Math.floor(Date.now() / 1000);
  const expSeconds = nowSeconds + accessTtlSeconds();

  return await new SignJWT({
    azp: clientId,
    gty: "client-credentials",
  })
    .setProtectedHeader({ alg: "EdDSA", kid })
    .setSubject(`${clientId}@clients`)
    .setIssuer(issuer)
    .setAudience(audience)
    .setIssuedAt(nowSeconds)
    .setExpirationTime(expSeconds)
    .sign(key);
}
```

- [ ] **Step 2: Create the verification module**

Create `packages/temper-cloud/src/oauth/machine-clients.ts`:

```ts
import { timingSafeEqual } from "node:crypto";
import type { NeonClient } from "../db.js";
import { hashToken } from "./mint.js";

/** Constant-time compare of two lowercase-hex strings of equal expected length. */
function hexEqual(a: string, b: string): boolean {
  const ba = Buffer.from(a, "hex");
  const bb = Buffer.from(b, "hex");
  return ba.length === bb.length && timingSafeEqual(ba, bb);
}

interface MachineSecretRow {
  secret_hash: string | null;
  secret_hash_previous: string | null;
  secret_previous_expires_at: string | Date | null;
}

/**
 * Verify a temper-issued client secret. True iff it matches the current secret, or the previous
 * secret while still inside its grace window. Only `issuer='temper'`, non-revoked rows are
 * considered — an `auth0-m2m` row (secret_hash NULL) never matches here; it verifies via JWKS.
 */
export async function verifyMachineSecret(
  db: NeonClient,
  clientId: string,
  clientSecret: string,
): Promise<boolean> {
  const rows = await db`
    SELECT secret_hash, secret_hash_previous, secret_previous_expires_at
    FROM kb_machine_clients
    WHERE client_id = ${clientId} AND issuer = 'temper' AND revoked_at IS NULL
  `;
  const row = rows[0] as MachineSecretRow | undefined;
  if (!row || !row.secret_hash) {
    return false;
  }

  const provided = hashToken(clientSecret);
  if (hexEqual(provided, row.secret_hash)) {
    return true;
  }

  if (
    row.secret_hash_previous &&
    row.secret_previous_expires_at &&
    new Date(row.secret_previous_expires_at) > new Date()
  ) {
    return hexEqual(provided, row.secret_hash_previous);
  }
  return false;
}

/**
 * Coarse liveness touch (mirrors the Rust gate's five-minute rule): writes only when
 * `last_seen_at` is NULL or older than five minutes, so token minting stays read-mostly.
 */
export async function touchMachineLastSeen(db: NeonClient, clientId: string): Promise<void> {
  await db`
    UPDATE kb_machine_clients
    SET last_seen_at = now()
    WHERE client_id = ${clientId}
      AND (last_seen_at IS NULL OR last_seen_at < now() - interval '5 minutes')
  `;
}
```

- [ ] **Step 3: Typecheck**

Run (from `packages/temper-cloud`): `bun run typecheck`
Expected: passes.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-cloud/src/oauth/mint.ts packages/temper-cloud/src/oauth/machine-clients.ts
git commit -m "feat(oauth): machine access-token minting + secret verification (Phase B1)"
```

---

### Task 9: TS — `client_credentials` grant + integration tests

**Files:**
- Modify: `packages/temper-cloud/src/oauth/endpoints.ts` (add the grant to `handleToken`)
- Create: `packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts`

**Interfaces:**
- Consumes: `verifyMachineSecret`, `touchMachineLastSeen`, `mintMachineAccessToken`, `accessTtlSeconds`.
- Produces: `POST /oauth/token` with `grant_type=client_credentials`.

- [ ] **Step 1: Write the failing integration test**

Create `packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts`:

```ts
import { createLocalJWKSet, exportPKCS8, generateKeyPair, jwtVerify } from "jose";
import type postgres from "postgres";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { NeonClient } from "../../../src/db.js";
import { handleToken } from "../../../src/oauth/endpoints.js";
import { getPublicJwks } from "../../../src/oauth/keys.js";
import { hashToken } from "../../../src/oauth/mint.js";
import { makeTestDb } from "../helpers/oauth-db.js";

function tokenRequest(body: Record<string, string>): Request {
  return new Request("https://as/oauth/token", {
    method: "POST",
    body: new URLSearchParams(body),
  });
}

/** Seed a temper-issued machine client with a known secret. Returns the client_id. */
async function seedTemperClient(
  sql: postgres.Sql,
  clientId: string,
  secret: string,
  opts: { previousSecret?: string; previousExpiresInSeconds?: number } = {},
): Promise<void> {
  const profileId = crypto.randomUUID();
  await sql`
    INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
    VALUES (${profileId}, ${`agent-${clientId}`}, ${`agent-${clientId}`}, NULL, '{}')
  `;
  const prevHash = opts.previousSecret ? hashToken(opts.previousSecret) : null;
  const prevExpiry =
    opts.previousExpiresInSeconds != null
      ? new Date(Date.now() + opts.previousExpiresInSeconds * 1000).toISOString()
      : null;
  await sql`
    INSERT INTO kb_machine_clients
      (client_id, issuer, label, profile_id, registered_by_profile_id,
       secret_hash, secret_hash_previous, secret_previous_expires_at)
    VALUES (${clientId}, 'temper', 'test', ${profileId}, ${profileId},
       ${hashToken(secret)}, ${prevHash}, ${prevExpiry})
  `;
}

describe("client_credentials grant", () => {
  let sql: postgres.Sql;
  let db: NeonClient;

  beforeAll(async () => {
    const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
    process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
    process.env.AS_SIGNING_KID = "test-kid-1";
    process.env.AS_ISSUER = "https://issuer.test";
    process.env.AS_AUDIENCE = "https://audience.test";
    process.env.AS_ACCESS_TTL_SECONDS = "900";
    ({ sql, db } = makeTestDb());
  });

  afterAll(async () => {
    await sql.end();
  });

  beforeEach(async () => {
    await sql`TRUNCATE kb_machine_clients CASCADE`;
    await sql`DELETE FROM kb_profiles WHERE handle LIKE 'agent-%'`;
  });

  it("mints a machine access token with the normalize_machine claim shape and no refresh token", async () => {
    await seedTemperClient(sql, "tmpr_cc1", "s3cr3t");

    const res = await handleToken(
      tokenRequest({ grant_type: "client_credentials", client_id: "tmpr_cc1", client_secret: "s3cr3t" }),
      db,
    );
    expect(res.status).toBe(200);
    const body = (await res.json()) as { access_token: string; token_type: string; expires_in: number; refresh_token?: string };
    expect(body.token_type).toBe("Bearer");
    expect(body.expires_in).toBe(900);
    expect(body.refresh_token).toBeUndefined();

    const jwks = createLocalJWKSet(await getPublicJwks());
    const { payload } = await jwtVerify(body.access_token, jwks, {
      issuer: "https://issuer.test",
      audience: "https://audience.test",
    });
    expect(payload.gty).toBe("client-credentials");
    expect(payload.azp).toBe("tmpr_cc1");
    expect(payload.sub).toBe("tmpr_cc1@clients");
    expect(payload.email).toBeUndefined();
  });

  it("rejects a wrong secret with invalid_client", async () => {
    await seedTemperClient(sql, "tmpr_cc2", "right");
    const res = await handleToken(
      tokenRequest({ grant_type: "client_credentials", client_id: "tmpr_cc2", client_secret: "wrong" }),
      db,
    );
    expect(res.status).toBe(401);
    expect((await res.json()).error).toBe("invalid_client");
  });

  it("rejects a revoked client", async () => {
    await seedTemperClient(sql, "tmpr_cc3", "s");
    await sql`UPDATE kb_machine_clients SET revoked_at = now() WHERE client_id = 'tmpr_cc3'`;
    const res = await handleToken(
      tokenRequest({ grant_type: "client_credentials", client_id: "tmpr_cc3", client_secret: "s" }),
      db,
    );
    expect(res.status).toBe(401);
  });

  it("accepts the previous secret within its grace window and rejects it after", async () => {
    // previous valid for another hour
    await seedTemperClient(sql, "tmpr_cc4", "new", { previousSecret: "old", previousExpiresInSeconds: 3600 });
    const ok = await handleToken(
      tokenRequest({ grant_type: "client_credentials", client_id: "tmpr_cc4", client_secret: "old" }),
      db,
    );
    expect(ok.status).toBe(200);

    // previous already expired
    await seedTemperClient(sql, "tmpr_cc5", "new", { previousSecret: "old", previousExpiresInSeconds: -1 });
    const expired = await handleToken(
      tokenRequest({ grant_type: "client_credentials", client_id: "tmpr_cc5", client_secret: "old" }),
      db,
    );
    expect(expired.status).toBe(401);
  });

  it("accepts credentials via HTTP Basic", async () => {
    await seedTemperClient(sql, "tmpr_cc6", "basic-secret");
    const basic = Buffer.from("tmpr_cc6:basic-secret").toString("base64");
    const res = await handleToken(
      new Request("https://as/oauth/token", {
        method: "POST",
        headers: { authorization: `Basic ${basic}` },
        body: new URLSearchParams({ grant_type: "client_credentials" }),
      }),
      db,
    );
    expect(res.status).toBe(200);
  });
});
```

- [ ] **Step 2: Run to confirm failure**

Run (from `packages/temper-cloud`, with Docker Postgres up):
`DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development bun run test:integration -- client-credentials`
Expected: FAIL — `unsupported_grant_type` (the grant isn't handled yet).

- [ ] **Step 3: Implement the grant**

In `packages/temper-cloud/src/oauth/endpoints.ts`:

(a) Extend the imports. Add to the `./mint.js` import (line 23) `mintMachineAccessToken`, and add a new import:

```ts
import { touchMachineLastSeen, verifyMachineSecret } from "./machine-clients.js";
```

(b) Add a machine-token response type and broaden `oauthJson` (lines 205-224). Add after `TokenResponse`:

```ts
/** The `/oauth/token` success body for client_credentials — no refresh token (RFC 6749 §4.4.3). */
interface MachineTokenResponse {
  access_token: string;
  token_type: "Bearer";
  expires_in: number;
}
```

and change `oauthJson`'s parameter type to include it:

```ts
function oauthJson(body: TokenResponse | MachineTokenResponse | OAuthErrorBody, status: number): Response {
```

(c) Add a credential reader near the other `handleToken` helpers (before `handleToken`, ~line 262):

```ts
/** Reads client credentials from HTTP Basic (preferred, RFC 6749 §2.3.1) or the form body. */
function readClientCredentials(
  req: Request,
  form: FormData,
): { clientId: string; clientSecret: string } | null {
  const auth = req.headers.get("authorization");
  if (auth?.startsWith("Basic ")) {
    const decoded = Buffer.from(auth.slice("Basic ".length), "base64").toString("utf8");
    const sep = decoded.indexOf(":");
    if (sep > 0) {
      return { clientId: decoded.slice(0, sep), clientSecret: decoded.slice(sep + 1) };
    }
  }
  const clientId = String(form.get("client_id") ?? "");
  const clientSecret = String(form.get("client_secret") ?? "");
  return clientId && clientSecret ? { clientId, clientSecret } : null;
}
```

(d) In `handleToken`, add the grant branch just before the final `return oauthError("unsupported_grant_type");` (line 301):

```ts
  if (grantType === "client_credentials") {
    const creds = readClientCredentials(req, form);
    if (!creds) {
      return oauthError("invalid_request");
    }
    if (!(await verifyMachineSecret(db, creds.clientId, creds.clientSecret))) {
      return oauthError("invalid_client", 401);
    }
    await touchMachineLastSeen(db, creds.clientId);
    const accessToken = await mintMachineAccessToken(creds.clientId);
    return oauthJson(
      { access_token: accessToken, token_type: "Bearer", expires_in: accessTtlSeconds() },
      200,
    );
  }
```

- [ ] **Step 4: Run the tests**

Run (from `packages/temper-cloud`): `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development bun run test:integration -- client-credentials`
Expected: 5 tests pass.

- [ ] **Step 5: Biome check**

Run (from `packages/temper-cloud`): `bun run check`
Expected: passes.

- [ ] **Step 6: Commit**

```bash
git add packages/temper-cloud/src/oauth/endpoints.ts packages/temper-cloud/tests/integration/oauth/client-credentials.test.ts
git commit -m "feat(oauth): client_credentials grant on the AS + integration tests (Phase B1)"
```

---

### Task 10: Rust e2e — a temper-issued token authenticates through the unchanged gate

**Files:**
- Modify: `tests/e2e/tests/machine_gate_e2e.rs` (HTTP surface: add a temper-issued case + a `register_temper_issued` helper)
- Modify: `tests/e2e/tests/auth_seam_m2m_e2e.rs` (MCP surface: add a temper-issued case)

**Interfaces:**
- Consumes: `common::setup`, `common::generate_machine_jwt`, the existing `register`/`machine_parts`/`build_mcp_service` helpers.

- [ ] **Step 1: Add the HTTP-surface test**

In `tests/e2e/tests/machine_gate_e2e.rs`, add a helper that seeds an `issuer='temper'` row (with a secret_hash, as issuance would) and a test asserting it passes the gate unchanged:

```rust
/// Register a temper-ISSUED machine client (issuer='temper', with a secret hash), as Phase B1's
/// `issue` path produces. The gate is issuer-agnostic, so this must authenticate exactly like an
/// auth0-m2m row.
async fn register_temper_issued(pool: &sqlx::PgPool, client_id: &str) -> Uuid {
    let profile_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
        profile_id,
        format!("agent-{client_id}"),
    )
    .execute(pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients \
           (client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
         VALUES ($1, 'temper', 'e2e', $2, $2, 'deadbeef')",
        client_id,
        profile_id,
    )
    .execute(pool)
    .await
    .expect("seed temper-issued registration");
    profile_id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn temper_issued_machine_reaches_the_data_plane(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    register_temper_issued(&pool, "tmpr_live").await;
    let token = common::generate_machine_jwt("tmpr_live");

    let response = reqwest::Client::new()
        .get(app.url("/api/resources"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        response.status(),
        200,
        "a temper-issued machine authenticates through the unchanged gate (D4)"
    );
}
```

- [ ] **Step 2: Add the MCP-surface test**

In `tests/e2e/tests/auth_seam_m2m_e2e.rs`, add a test that seeds an `issuer='temper'` row and drives `ensure_profile_from_parts` with the forged machine claims (reuse the file's existing `build_mcp_service` and `machine_parts` helpers; mirror the existing inline seed but set `issuer='temper'` and a `secret_hash`):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn temper_issued_machine_resolves_on_mcp(pool: sqlx::PgPool) {
    let svc = build_mcp_service(&pool).await;

    let profile_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, 'agent-tmpr-mcp', 'agent-tmpr-mcp', NULL, '{}')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed profile");
    sqlx::query!(
        "INSERT INTO kb_machine_clients \
           (client_id, issuer, label, profile_id, registered_by_profile_id, secret_hash) \
         VALUES ('tmpr_mcp', 'temper', 'e2e', $1, $1, 'deadbeef')",
        profile_id,
    )
    .execute(&pool)
    .await
    .expect("seed temper-issued registration");

    let result = svc.ensure_profile_from_parts(&machine_parts("tmpr_mcp")).await;
    assert!(
        result.is_ok(),
        "a temper-issued machine resolves on the MCP surface too (D4): {result:?}"
    );
}
```

(If the existing MCP test uses a different assertion entry point than `ensure_profile_from_parts`, mirror whatever the sibling test in this file does — copy its call shape verbatim.)

- [ ] **Step 3: Run the e2e tests**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-e2e --features test-db -E 'binary(machine_gate_e2e) + binary(auth_seam_m2m_e2e)' --test-threads 1`
Expected: all machine tests pass (the two new ones plus the Phase A ones).

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/machine_gate_e2e.rs tests/e2e/tests/auth_seam_m2m_e2e.rs
git commit -m "test(e2e): temper-issued machine authenticates through the unchanged gate (Phase B1)"
```

---

### Task 11: Regenerate sqlx caches + full verification

**Files:**
- Modify: `.sqlx/` (workspace), `crates/temper-services/.sqlx/`, `crates/temper-api/.sqlx/`, `tests/e2e/.sqlx/` (regenerated caches)

**Interfaces:**
- Produces: committed offline sqlx caches so `SQLX_OFFLINE=true` builds (CI, `cargo make check`) resolve the new queries.

- [ ] **Step 1: Regenerate the workspace cache**

Run (with Docker Postgres up and the migration applied):
`DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace -- --all-features`
Expected: `.sqlx/` updated with the new `issue`/`rotate_secret` query entries.

- [ ] **Step 2: Regenerate the per-crate test-target caches**

Run in order:
```bash
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
```
Expected: each rewrites its crate's `.sqlx/` cache (test-target queries for the new seeds/tests). Note: `prepare-api`/`prepare-services` also materialize many untracked `.sqlx` files — **do not `git add .sqlx` wholesale**; stage only the tracked cache dirs that changed for the queries you added (`git add -p` or add specific files; check `git status` for which committed entries changed).

- [ ] **Step 3: Full offline check**

Run: `cargo make check`
Expected: fmt, clippy (`-D warnings`), docs, machete, OpenAPI, TS typecheck, biome all green. This is the honest offline probe — it uses `SQLX_OFFLINE=true`, so a missing cache entry fails here.

- [ ] **Step 4: Run the full Rust DB test tier**

Run: `cargo make test-db`
Expected: the whole `test-db` tier passes, including the new service tests and e2e tests.

- [ ] **Step 5: Run the TS integration suite**

Run (from `packages/temper-cloud`): `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development bun run test:integration`
Expected: all oauth integration tests pass, including the new `client-credentials.test.ts`.

- [ ] **Step 6: Commit the caches**

```bash
git add .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx tests/e2e/.sqlx
git commit -m "chore(sqlx): regenerate offline caches for Phase B1 queries"
```

---

## Self-Review Notes

- **Spec coverage:** §1 schema → Task 1; §2 issuance (Rust service + CLI + API) → Tasks 2–7; §3 verification (TS grant) → Tasks 8–9; §4 rotation → Task 5 (+ verified in Task 9's grace test); §5 coverage split → TS integration (Task 9) + Rust `test-db`/e2e both surfaces (Task 10); D1 SHA-256 → Task 2; D4 verifier-untouched → asserted by Task 10; D5 `auth0-m2m` link → asserted in Task 4; D9 admin gate → Task 6 handlers.
- **Deferred (not in this plan, per spec):** B2 authz widening; steward repointing; the `expires_in`-only response is intentional (no refresh token).
- **Ordering:** Task 1 (migration) must run first (later queries reference the new columns). Task 3 (types + `apply_reach` slices) precedes Tasks 4/6/7 that consume the types. Task 11 (caches) is last, after all SQL exists.
- **Deploy note (out of plan, for the PR/rollout):** Phase A's migration and B1's must be applied to prod **before** the B1 code deploys (additive migrate-then-deploy). Re-verify the backfill/steward set before migrating prod.
