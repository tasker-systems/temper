# SAML SP Phase 1 — Temper Authorization Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Temper a native SAML Service Provider by standing up a minimal OAuth 2.0 Authorization Server in `temper-cloud` whose upstream authentication method is SAML and whose output is a short-lived EdDSA-signed Temper JWT that `temper-api` trusts and `resolve_from_claims` consumes unchanged.

**Architecture:** SAML terminates in `temper-cloud` (the Neon-backed `api/` Vercel TS functions). A minimal OAuth AS there exposes `/oauth/authorize` + `/oauth/token` (auth-code + PKCE + refresh) + `/oauth/jwks` + RFC 8414 metadata, with `/oauth/saml/{login,acs,metadata}` as the upstream. UI, CLI, and MCP are all ordinary auth-code+PKCE clients; Phase 1 ships UI + CLI. `temper-api` trusts the AS as its single issuer after one small change: adding `Algorithm::EdDSA` to the JWT validation allow-list.

**Tech Stack:** TypeScript (`packages/temper-cloud`, `@vercel` functions, `@neondatabase/serverless`, `jose@6`, `@node-saml/node-saml`), SvelteKit (`packages/temper-ui`), Rust (`temper-services`/`temper-api`, `jsonwebtoken`), Postgres (Neon, sqlx migrations), Vitest, cargo-nextest.

## Global Constraints

- **Additive-only on `main`** — all schema migrations merging to `main` are backward-compatible/additive (root `CLAUDE.md`, `DEPLOYING.md`). No edits to shipped migrations.
- **Migrations version-portable PG17 (Neon)/PG18 (local)** — no version-specific SQL. Conventions: `id UUID PRIMARY KEY DEFAULT uuid_generate_v7()`, `TIMESTAMPTZ NOT NULL DEFAULT now()`, inline `idx_<table>_<cols>` indexes, no RLS/GRANT.
- **Typed structs over inline JSON** (Rust) / typed request-response shapes (TS). No `serde_json::json!()` for known shapes.
- **Persistence layer** — TS DB access via `getDb()` (`@neondatabase/serverless`) with parameterized tagged-template queries only; no string interpolation into SQL. Phase 1 adds **no Rust `sqlx` query** to any new table, so no `.sqlx` cache regeneration.
- **Auth before writes**; short-lived access tokens (~15 min); refresh tokens stored hashed, single-use-rotatable; assertion IDs replay-protected.
- **No secrets in URLs or logs.** Signing key + session secret from Vercel env only.
- **Pino logging** in TS (`packages/temper-cloud/src/logger.ts`); no `console.log`.
- **Biome** (TS) + **clippy `-D warnings`** + `cargo fmt` (Rust) must pass; run `cargo make check` / `bun run check` before each commit.
- **cargo fmt** is part of the pre-commit gate — run it in every Rust task's commit step.

---

## File Structure

**Rust (M0):**
- Modify `crates/temper-services/src/state.rs` — extend `validation()` allow-list; add EdDSA unit test.
- Modify `tests/e2e/tests/common/mod.rs` — add Ed25519 fixture loader + `generate_test_jwt_eddsa`.
- Create `tests/e2e/tests/fixtures/test_ed25519.pkcs8` + `.jwk` — Ed25519 test keypair.
- Create `tests/e2e/tests/eddsa_auth_test.rs` — EdDSA token → `require_auth` → `resolve_from_claims`.

**Migrations (M1/M2):**
- Create `migrations/2026070100000X_saml_as_tables.sql` — `kb_saml_idp`, `kb_oauth_flow`, `kb_oauth_refresh_tokens`, `kb_saml_replay`.

**TypeScript — business logic (`packages/temper-cloud/src/`):**
- `oauth/keys.ts` — Ed25519 signing key load (`importPKCS8`) + public JWKS (`exportJWK`).
- `oauth/mint.ts` — `mintAccessToken` / `mintRefreshToken` / `hashToken`.
- `oauth/pkce.ts` — PKCE S256 verification (pure).
- `oauth/flow.ts` — DB ops for `kb_oauth_flow` (create pending, bind code, consume) + refresh-token store ops.
- `oauth/metadata.ts` — RFC 8414 authorization-server metadata builder (pure).
- `saml/config.ts` — load active `kb_saml_idp` row → `SamlConfig`.
- `saml/sp.ts` — `buildLoginRedirect`, `validateAssertion` (wraps node-saml), `buildSpMetadata`, `mapProfileToClaims`.
- `saml/replay.ts` — assertion-ID replay guard.

**TypeScript — Vercel function entrypoints (`api/oauth/`):**
- `api/oauth/jwks.ts`, `api/oauth/authorization-server.ts` (served at `/.well-known/oauth-authorization-server`), `api/oauth/authorize.ts`, `api/oauth/token.ts`, `api/oauth/saml/login.ts`, `api/oauth/saml/acs.ts`, `api/oauth/saml/metadata.ts`.
- Modify root `vercel.json` — route the new `/oauth/*` + metadata paths to these functions before the MCP catch.

**SvelteKit UI (M4):**
- Modify `packages/temper-ui/src/lib/server/oidc.ts` / `oidc-core.ts` config resolution — allow pointing at the Temper AS via `OIDC_*` env (already generic; mainly env + metadata wiring).
- No structural route change — `/auth/login` + `/auth/callback` already do auth-code+PKCE.

**CLI + docs (M4):**
- Modify `crates/temper-cli/src/commands/init.rs` — add a SAML/AS provider preset writer.
- Create `docs/guides/self-hosting-saml.md`; modify `docs/guides/self-hosting-okta.md` to cross-reference.

---

## Milestone M0 — `temper-api` accepts EdDSA (independently shippable)

### Task 0.1: Make JWT validation algorithm-aware (accept EdDSA without breaking RS256)

> **Correction (discovered in implementation):** `jsonwebtoken` 9's `verify_signature` runs `for alg in &validation.algorithms { if key.family != alg.family() { return InvalidAlgorithm } }`. So a mixed `vec![RS256, EdDSA]` allow-list against the single cached key **fails for every token** (one of the two families never matches the key) — it would break the live RS256 path, not just fail to add EdDSA. The correct fix: the allow-list must contain **only the algorithm matching the loaded key's family**. Thread the key's algorithm from key-load through `validation()`.

**Files:**
- Modify: `crates/temper-services/src/state.rs` — `CachedKeys`, `refresh()`, `with_static_key()`, `get_decoding_key()`, `validation()`; add unit test.
- Modify: `crates/temper-api/src/middleware/auth.rs:70-83` (`require_auth` call site).
- Modify: `crates/temper-mcp/src/middleware.rs:42-57` (MCP call site).
- Modify: `tests/e2e/tests/common/mod.rs` (the `with_static_key(...)` call in `setup`, ~`:236`).

**Interfaces:**
- Produces:
  - `pub struct VerificationKey { pub key: DecodingKey, pub algorithm: Algorithm }` (in `state.rs`).
  - `get_decoding_key(&self) -> Result<VerificationKey, String>` (return type changed from `DecodingKey`).
  - `validation(&self, issuer: &str, audience: Option<&str>, algorithm: Algorithm) -> Validation` (added `algorithm` param; `algorithms = vec![algorithm]`).
  - `with_static_key(key: DecodingKey, algorithm: Algorithm) -> Self` (added `algorithm` param).
- Consumes: existing Ed25519 test helpers in the `state.rs` `#[cfg(test)]` module (the keypair scaffold near `:187-276` that builds an Ed25519 `EncodingKey`/`DecodingKey` and encodes with `Header::new(Algorithm::EdDSA)`). Read the module and reuse its actual helper names.

- [ ] **Step 1: Write the failing test** (exercises the PRODUCTION `validation()` with the key's algorithm)

```rust
#[test]
fn validation_accepts_eddsa_token_for_eddsa_key() {
    use jsonwebtoken::{encode, decode, Header, Algorithm};
    // reuse the existing module helper that yields an Ed25519 (EncodingKey, DecodingKey)
    let (encoding_key, decoding_key) = /* existing ed25519 helper */;
    let store = JwksKeyStore::with_static_key(decoding_key.clone(), Algorithm::EdDSA);

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Claims { sub: String, iss: String, aud: String, exp: usize, iat: usize }
    let now = 1_900_000_000usize;
    let claims = Claims { sub: "u1".into(), iss: "https://as.example".into(),
        aud: "https://api.example".into(), exp: now + 3600, iat: now };
    let token = encode(&Header::new(Algorithm::EdDSA), &claims, &encoding_key).unwrap();

    let vk = /* block_on */ store.get_decoding_key(); // returns VerificationKey; algorithm == EdDSA
    let validation = store.validation("https://as.example", Some("https://api.example"), Algorithm::EdDSA);
    assert!(decode::<Claims>(&token, &decoding_key, &validation).is_ok());
}
```

(`get_decoding_key` is async; in the sync test either assert `store.validation(..., Algorithm::EdDSA)` directly, or wrap the async call. Keep the test focused on: EdDSA key + EdDSA-scoped validation ⇒ decode Ok.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-services validation_accepts_eddsa_token_for_eddsa_key`
Expected: FAIL to compile first (signatures changed), then FAIL `InvalidAlgorithm` until the fix lands.

- [ ] **Step 3: Implement the algorithm-aware store**

- Add `algorithm: Algorithm` to `CachedKeys`.
- `refresh()`: derive the algorithm from the chosen JWK (`AlgorithmParameters::RSA(_) => Algorithm::RS256`, `OctetKeyPair(Ed25519) => Algorithm::EdDSA`) and store it alongside the key.
- `with_static_key(key, algorithm)`: store both.
- `get_decoding_key()`: return `VerificationKey { key, algorithm }`.
- `validation(issuer, audience, algorithm)`: `let mut v = Validation::new(algorithm); v.algorithms = vec![algorithm];` then issuer/audience unchanged.
- Update the two call sites to `let vk = ...get_decoding_key().await?; let validation = ...validation(issuer, audience, vk.algorithm); decode(&token, &vk.key, &validation)`.
- Update `tests/e2e/tests/common/mod.rs`'s `with_static_key(rsa_key)` → `with_static_key(rsa_key, Algorithm::RS256)`.
- `grep -rn "with_static_key\|get_decoding_key\|\.validation(" crates/ tests/` and fix every caller.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services -E 'test(validation)'` (new test + existing validation tests). Expected: PASS. The existing e2e RS256 path is the regression guard for RS256 (its harness now passes `Algorithm::RS256`).

- [ ] **Step 5: Controller gate + commit** (split-labor: the implementer stops after Step 4 and reports; the controller runs `cargo fmt` + `cargo make check` + commits, to avoid the pre-commit-hook auto-background stall)

```bash
cargo fmt && cargo make check
git add crates/temper-services/src/state.rs crates/temper-api/src/middleware/auth.rs crates/temper-mcp/src/middleware.rs tests/e2e/tests/common/mod.rs
git commit -m "fix(services): make JWT validation allow-list track the loaded key's algorithm (accept EdDSA)"
```

### Task 0.2: E2E — an EdDSA token authenticates and resolves a profile

**Files:**
- Create: `tests/e2e/tests/fixtures/test_ed25519.pkcs8`, `tests/e2e/tests/fixtures/test_ed25519.pub.jwk`
- Modify: `tests/e2e/tests/common/mod.rs` — add `setup_eddsa(pool)` + `generate_test_jwt_eddsa(sub, email)`
- Create: `tests/e2e/tests/eddsa_auth_test.rs`

**Interfaces:**
- Consumes: existing `setup(pool)` shape (`common/mod.rs:229`) building `AppState::new(pool, jwks_store, api_config)`; `JwksKeyStore::with_static_key`; `generate_test_jwt` (`:129`) as the RS256 analog.
- Produces: `generate_test_jwt_eddsa(sub: &str, email: &str) -> String`; `setup_eddsa(pool) -> TestApp` (an EdDSA-keyed variant of `setup`).

- [ ] **Step 1: Generate the fixture keypair**

```bash
openssl genpkey -algorithm ed25519 -out tests/e2e/tests/fixtures/test_ed25519.pkcs8
# public JWK is produced in Step 3's helper via jsonwebtoken; store the PEM pub too:
openssl pkey -in tests/e2e/tests/fixtures/test_ed25519.pkcs8 -pubout -out tests/e2e/tests/fixtures/test_ed25519.pub.pem
```

- [ ] **Step 2: Write the failing e2e test**

Create `tests/e2e/tests/eddsa_auth_test.rs`:

```rust
mod common;
use common::setup_eddsa;

#[sqlx::test(migrations = "../../migrations")]
async fn eddsa_token_authenticates_and_resolves_profile(pool: sqlx::PgPool) {
    let app = setup_eddsa(pool).await;
    let token = common::generate_test_jwt_eddsa("eddsa-user", "eddsa@test.example");
    let res = app.client
        .get(format!("{}/api/profile", app.base_url))
        .bearer_auth(token)
        .send().await.unwrap();
    assert_eq!(res.status(), 200, "EdDSA-authenticated /api/profile must succeed");
}
```

- [ ] **Step 3: Add the EdDSA helpers to `common/mod.rs`**

Mirror `generate_test_jwt` (`:129`) and `setup` (`:229`) but with `Algorithm::EdDSA`, `EncodingKey::from_ed_pem(include_bytes!("../fixtures/test_ed25519.pkcs8"))`, and `DecodingKey::from_ed_pem(include_bytes!("../fixtures/test_ed25519.pub.pem"))` injected via `JwksKeyStore::with_static_key(decoding_key, Algorithm::EdDSA)` (the algorithm param added in Task 0.1). Set `ApiConfig { auth_issuer: "test-issuer", auth_audience: None, .. }` exactly as `setup` does so the token's `iss` matches.

- [ ] **Step 4: Run the e2e**

Run: `cargo make test-e2e` (or `cargo nextest run -p e2e --test eddsa_auth_test`)
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo make check
git add tests/e2e/tests/eddsa_auth_test.rs tests/e2e/tests/common/mod.rs tests/e2e/tests/fixtures/test_ed25519.*
git commit -m "test(e2e): EdDSA-signed token authenticates and resolves a profile"
```

---

## Milestone M1 — AS token-issuer core (`temper-cloud`)

> Riskiest novel machinery first: EdDSA signing + JWKS publication, verified end-to-end against M0's Rust before any endpoints exist.

### Task 1.0: Add dependencies

**Files:** Modify `packages/temper-cloud/package.json`.

- [ ] **Step 1:** Add deps and install:

```bash
cd packages/temper-cloud
bun add @node-saml/node-saml
# jose@^6 and @neondatabase/serverless already present
bun run typecheck
```

- [ ] **Step 2: Commit**

```bash
git add packages/temper-cloud/package.json bun.lock
git commit -m "chore(temper-cloud): add @node-saml/node-saml dependency"
```

### Task 1.1: PKCE verification (pure)

**Files:**
- Create: `packages/temper-cloud/src/oauth/pkce.ts`
- Test: `packages/temper-cloud/src/oauth/pkce.test.ts`

**Interfaces:**
- Produces: `verifyPkceS256(verifier: string, challenge: string): boolean`.

- [ ] **Step 1: Failing test**

```ts
import { describe, it, expect } from "vitest";
import { verifyPkceS256 } from "./pkce.js";
describe("verifyPkceS256", () => {
  it("accepts a matching S256 verifier/challenge pair", () => {
    // challenge = base64url(sha256(verifier)); precomputed for "test-verifier-12345..."
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    expect(verifyPkceS256(verifier, challenge)).toBe(true);
    expect(verifyPkceS256("wrong", challenge)).toBe(false);
  });
});
```

- [ ] **Step 2:** Run `bun run test src/oauth/pkce.test.ts` → FAIL (module missing).

- [ ] **Step 3: Implement**

```ts
import { createHash } from "node:crypto";
export function verifyPkceS256(verifier: string, challenge: string): boolean {
  const computed = createHash("sha256").update(verifier).digest("base64url");
  // constant-time compare
  const a = Buffer.from(computed);
  const b = Buffer.from(challenge);
  return a.length === b.length && require("node:crypto").timingSafeEqual(a, b);
}
```

- [ ] **Step 4:** Run test → PASS.
- [ ] **Step 5:** Commit: `feat(oauth): PKCE S256 verification helper`.

### Task 1.2: Signing key + public JWKS

**Files:**
- Create: `packages/temper-cloud/src/oauth/keys.ts`
- Test: `packages/temper-cloud/src/oauth/keys.test.ts`

**Interfaces:**
- Consumes env: `AS_SIGNING_KEY_PKCS8` (Ed25519 PKCS8 PEM), `AS_SIGNING_KID` (string).
- Produces: `getSigningKey(): Promise<{ key: CryptoKey; kid: string }>`; `getPublicJwks(): Promise<{ keys: JWK[] }>` (each key annotated `alg:"EdDSA"`, `use:"sig"`, `kid`).

- [ ] **Step 1: Failing test** (generate an ephemeral key in-test, set env, assert the exported JWK is EdDSA/OKP with the kid and no `d`):

```ts
import { describe, it, expect, beforeAll } from "vitest";
import { generateKeyPair, exportPKCS8 } from "jose";
beforeAll(async () => {
  const { privateKey } = await generateKeyPair("Ed25519", { extractable: true });
  process.env.AS_SIGNING_KEY_PKCS8 = await exportPKCS8(privateKey);
  process.env.AS_SIGNING_KID = "test-kid-1";
});
it("publishes an EdDSA public JWKS without private material", async () => {
  const { getPublicJwks } = await import("./keys.js");
  const jwks = await getPublicJwks();
  expect(jwks.keys[0]).toMatchObject({ kty: "OKP", crv: "Ed25519", alg: "EdDSA", use: "sig", kid: "test-kid-1" });
  expect(jwks.keys[0]).not.toHaveProperty("d");
});
```

- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3: Implement** using `importPKCS8(pem, "EdDSA")`, `exportJWK(publicKeyFromPrivate)` (derive public via `createPublicKey` from `node:crypto` or import the PKCS8 and export the public JWK), annotate `alg/use/kid`. Cache the imported key in module scope.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(oauth): Ed25519 signing key load + public JWKS`.

### Task 1.3: Mint access + refresh tokens

**Files:**
- Create: `packages/temper-cloud/src/oauth/mint.ts`
- Test: `packages/temper-cloud/src/oauth/mint.test.ts`

**Interfaces:**
- Consumes: `getSigningKey()`; env `AS_ISSUER`, `AS_AUDIENCE`, `AS_ACCESS_TTL_SECONDS` (default 900).
- Produces:
  - `type MintedClaims = { sub: string; email: string; email_verified: boolean }`
  - `mintAccessToken(claims: MintedClaims): Promise<string>` — EdDSA JWT, header `{alg:"EdDSA", kid}`, payload `{sub, email, email_verified, iss, aud, iat, exp}`.
  - `newOpaqueToken(): string` (32-byte base64url) and `hashToken(t: string): string` (sha256 hex) for refresh tokens/codes.

- [ ] **Step 1: Failing test** — mint, then verify with the public JWKS (`createLocalJWKSet` + `jwtVerify`) asserting alg EdDSA, `iss`/`aud`, and the claim passthrough:

```ts
it("mints an EdDSA access token verifiable via the public JWKS", async () => {
  const { mintAccessToken } = await import("./mint.js");
  const { getPublicJwks } = await import("./keys.js");
  const jwt = await mintAccessToken({ sub: "u1", email: "u1@x.io", email_verified: true });
  const JWKS = createLocalJWKSet(await getPublicJwks());
  const { payload, protectedHeader } = await jwtVerify(jwt, JWKS, { issuer: process.env.AS_ISSUER, audience: process.env.AS_AUDIENCE });
  expect(protectedHeader.alg).toBe("EdDSA");
  expect(payload).toMatchObject({ sub: "u1", email: "u1@x.io", email_verified: true });
});
```

- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3: Implement** with `new SignJWT({email, email_verified}).setProtectedHeader({alg:"EdDSA", kid}).setSubject(sub).setIssuer(AS_ISSUER).setAudience(AS_AUDIENCE).setIssuedAt().setExpirationTime(...).sign(key)`.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(oauth): mint EdDSA access tokens + opaque token helpers`.

### Task 1.4: Cross-language proof — mint in TS, validate in Rust

**Files:**
- Create: `tests/e2e/tests/fixtures/gen_temper_token.mjs` (a tiny node script that imports mint with a fixed fixture key) OR reuse the shared fixture key.
- Test: extend `tests/e2e/tests/eddsa_auth_test.rs` with a case that loads a token minted by the TS path.

> This locks the wire contract: a token minted by `mint.ts` (using the shared fixture Ed25519 key) must pass `require_auth`. Use the SAME `test_ed25519.pkcs8` fixture in both `keys.ts` (via env in the script) and the Rust `with_static_key` decoding key.

- [ ] **Step 1:** Write a test that shells out to the mjs mint script (with `AS_SIGNING_KEY_PKCS8` = the fixture PEM, `AS_ISSUER=test-issuer`, `AS_AUDIENCE` matching the Rust config), captures the JWT, and asserts `/api/profile` returns 200 under `setup_eddsa`.
- [ ] **Step 2:** Run → FAIL (script/flow absent).
- [ ] **Step 3:** Implement the mjs script (imports `mintAccessToken`, prints the JWT).
- [ ] **Step 4:** Run → PASS. This proves TS-minted EdDSA ⇄ Rust `require_auth`.
- [ ] **Step 5:** Commit: `test(e2e): TS-minted EdDSA token validated by require_auth`.

---

## Milestone M2 — AS state schema + OAuth endpoints wired to SAML (functional login)

> M2 and the SAML upstream are built together (no stub, per decision): the PR is a working SAML login. Internal tasks stay independently testable (schema, pure mappers, canned-assertion validation) with the mock-IdP e2e last.

### Task 2.1: Additive migration — AS state + IdP config tables

**Files:** Create `migrations/2026070100000X_saml_as_tables.sql` (use the next daily sequence).

**Interfaces (columns other tasks rely on):**
- `kb_saml_idp(idp_key TEXT PK, is_active BOOL, idp_cert TEXT, idp_sso_url TEXT, idp_entity_id TEXT, sp_entity_id TEXT, acs_url TEXT, nameid_format TEXT, email_attr TEXT, stable_id_attr TEXT, created, updated)`.
- `kb_oauth_flow(id UUID PK, relay_state TEXT UNIQUE, code_hash TEXT UNIQUE, status TEXT CHECK IN ('pending_saml','code_issued','consumed'), client_id TEXT, redirect_uri TEXT, code_challenge TEXT, code_challenge_method TEXT, oauth_state TEXT, audience TEXT, claims JSONB, created, expires_at)`.
- `kb_oauth_refresh_tokens(id UUID PK, token_hash TEXT UNIQUE, client_id TEXT, claims JSONB, created, expires_at, revoked_at, rotated_to UUID)`.
- `kb_saml_replay(assertion_id TEXT PK, expires_at TIMESTAMPTZ)`.

- [ ] **Step 1:** Write the migration following `migrations/20260630000001_access_grants_seam.sql` style (uuid_generate_v7(), TIMESTAMPTZ DEFAULT now(), inline `idx_kb_oauth_flow_relay_state`, `idx_kb_oauth_flow_code_hash`, `idx_kb_oauth_refresh_tokens_token_hash`, `idx_kb_saml_replay_expires`). No RLS/GRANT. Header comment: additive, namespace-free.
- [ ] **Step 2:** Apply locally: `cargo make docker-up && sqlx migrate run` (DATABASE_URL set). Expected: applies cleanly on PG18.
- [ ] **Step 3:** Verify idempotent forward-only + that `cargo make check` (SQLX_OFFLINE) still passes (no Rust query touches these, so no cache change).
- [ ] **Step 4:** Commit: `feat(db): SAML IdP + OAuth AS state tables (additive)`.

### Task 2.2: `kb_saml_idp` → `SamlConfig` loader

**Files:** Create `packages/temper-cloud/src/saml/config.ts` + `.test.ts`.

**Interfaces:**
- Produces: `loadActiveIdp(db: NeonClient): Promise<SamlIdpRow | null>`; `toSamlConfig(row: SamlIdpRow): SamlConfig` (shape `{callbackUrl, entryPoint, issuer, idpCert, audience, identifierFormat, wantAssertionsSigned: true, validateInResponseTo: "never"}`).
- `SamlIdpRow` mirrors the table columns.

- [ ] **Step 1: Failing test** — given a fake row, `toSamlConfig` maps `acs_url→callbackUrl`, `idp_sso_url→entryPoint`, `sp_entity_id→issuer`, `idp_cert→idpCert`, `sp_entity_id→audience`, `nameid_format→identifierFormat`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement `toSamlConfig` (pure) + `loadActiveIdp` (tagged-template `SELECT ... WHERE is_active = true LIMIT 1`).
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(saml): active-IdP config loader`.

### Task 2.3: Assertion → claims mapping (pure)

**Files:** Create `packages/temper-cloud/src/saml/sp.ts` (add `mapProfileToClaims`) + test.

**Interfaces:**
- Consumes: node-saml `Profile` (`{nameID, nameIDFormat, attributes, ...}`), the `SamlIdpRow` (for `email_attr`/`stable_id_attr`).
- Produces: `mapProfileToClaims(profile: Profile, idp: SamlIdpRow): MintedClaims` — `sub` = persistent NameID (when `nameIDFormat` endsWith `:persistent`) else `attributes[idp.stable_id_attr]`; throws if neither present. `email` = `attributes[idp.email_attr]` (or email-format nameID). `email_verified` = `true`.

- [ ] **Step 1: Failing test** — three cases: persistent NameID → sub=nameID; transient NameID + stable attr → sub=attr; neither → throws. All set `email_verified:true`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement (pure function; no I/O).
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(saml): map SAML profile to Temper AuthClaims`.

### Task 2.4: Assertion validation wrapper + replay guard

**Files:** `packages/temper-cloud/src/saml/sp.ts` (`validateAssertion`), `packages/temper-cloud/src/saml/replay.ts`, tests with a canned signed assertion fixture.

**Interfaces:**
- Produces:
  - `buildLoginRedirect(idp: SamlIdpRow, relayState: string): Promise<string>` — `new SAML(toSamlConfig(idp)).getAuthorizeUrlAsync(relayState, undefined, {})`.
  - `validateAssertion(idp, samlResponseB64): Promise<{ profile: Profile; assertionId: string }>` — `new SAML(cfg).validatePostResponseAsync({SAMLResponse})`; throws on bad signature/audience/timing (node-saml enforces the checklist); extracts assertion ID.
  - `buildSpMetadata(idp): string` — `generateServiceProviderMetadata()`.
  - `guardReplay(db, assertionId, expiresAt): Promise<void>` — `INSERT INTO kb_saml_replay ... ON CONFLICT DO NOTHING`; throw `ReplayError` if 0 rows inserted.

- [ ] **Step 1:** Generate a canned self-signed IdP cert + a signed SAMLResponse fixture (script under `packages/temper-cloud/test-fixtures/`), and write tests: valid → returns profile; tampered → throws; replayed assertionId → `guardReplay` throws on 2nd call (use a test Neon/pglite or mock the db to return rowCount 0).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement the wrappers + replay guard.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(saml): assertion validation + replay guard`.

### Task 2.5: OAuth flow store ops

**Files:** `packages/temper-cloud/src/oauth/flow.ts` + test.

**Interfaces:**
- `createPendingFlow(db, {relayState, clientId, redirectUri, codeChallenge, codeChallengeMethod, oauthState, audience, expiresAt}): Promise<void>`
- `bindCodeToFlow(db, relayState, {code, claims, expiresAt}): Promise<{ redirectUri, oauthState }>` — status `pending_saml`→`code_issued`, stores `code_hash`+`claims`.
- `consumeCode(db, code, codeVerifier): Promise<MintedClaims>` — looks up by `code_hash`, checks unexpired + status `code_issued`, verifies PKCE via `verifyPkceS256`, sets `consumed`, returns claims. Throws on any failure.
- Refresh store: `storeRefreshToken(db, {token, clientId, claims, expiresAt})`, `rotateRefreshToken(db, token): Promise<MintedClaims>` (single-use rotation), `revoke`.

- [ ] **Step 1:** Failing tests for the state transitions + PKCE gate + single-use (second `consumeCode` throws; rotated refresh token invalidates the old).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement with parameterized tagged-template queries.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(oauth): authz-code + refresh-token flow store`.

### Task 2.6: RFC 8414 metadata + JWKS endpoints

**Files:** `packages/temper-cloud/src/oauth/metadata.ts` + test; `api/oauth/jwks.ts`, `api/oauth/authorization-server.ts`; modify `vercel.json`.

**Interfaces:**
- `buildAsMetadata(issuer: string): AsMetadata` — `{issuer, authorization_endpoint, token_endpoint, jwks_uri, response_types_supported:["code"], grant_types_supported:["authorization_code","refresh_token"], code_challenge_methods_supported:["S256"], token_endpoint_auth_methods_supported:["none"]}`.

- [ ] **Step 1:** Failing test for `buildAsMetadata` field values (pure).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement the builder; wire the two Vercel functions (`jwks.ts` returns `getPublicJwks()`; `authorization-server.ts` returns `buildAsMetadata(AS_ISSUER)`). Add `vercel.json` routes for `/oauth/jwks`, `/.well-known/oauth-authorization-server` **before** the existing `/oauth`/`/.well-known`→`/api/mcp` catch. **Decision:** on a SAML instance the AS's metadata is authoritative; the MCP function's Auth0 metadata is unaffected on Auth0 instances because those instances do not deploy the AS routes (guarded by presence of `AS_ISSUER`). Document this in the function header.
- [ ] **Step 4:** Run test + `bun run check` → PASS.
- [ ] **Step 5:** Commit: `feat(oauth): RFC 8414 metadata + JWKS endpoints + routing`.

### Task 2.7: `/oauth/authorize` + `/oauth/saml/{login,acs,metadata}`

**Files:** `api/oauth/authorize.ts`, `api/oauth/saml/login.ts`, `api/oauth/saml/acs.ts`, `api/oauth/saml/metadata.ts`; `vercel.json` routes.

**Flow (wire the pieces from 2.2–2.5):**
- `authorize.ts` (GET): validate `response_type=code`, `client_id`, `redirect_uri`, `code_challenge`, `code_challenge_method=S256`, `state`; generate a `relayState` nonce; `createPendingFlow(...)`; 302 → `/oauth/saml/login?rs=<relayState>`.
- `saml/login.ts` (GET): `loadActiveIdp`; `buildLoginRedirect(idp, relayState)`; 302 to the IdP.
- `saml/acs.ts` (POST): read `SAMLResponse` + `RelayState`; `loadActiveIdp`; `validateAssertion`; `guardReplay`; `mapProfileToClaims`; `newOpaqueToken()` → `bindCodeToFlow(relayState, {code, claims, ...})`; 302 → `redirectUri?code=<code>&state=<oauthState>`.
- `saml/metadata.ts` (GET): `buildSpMetadata(idp)` as `application/xml`.

- [ ] **Step 1:** Write an integration test (Vitest) driving `authorize→login` producing a redirect to the IdP with a persisted pending flow; and an `acs` test that, given a canned valid `SAMLResponse` + matching `RelayState`, issues a code and 302s to the redirect_uri.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement the four functions (thin: parse → call src/ logic → respond) + `vercel.json` routes ordered before the MCP catch.
- [ ] **Step 4:** Run → PASS; `bun run check`.
- [ ] **Step 5:** Commit: `feat(oauth): authorize + SAML login/acs/metadata endpoints`.

### Task 2.8: `/oauth/token` (code + refresh grants)

**Files:** `api/oauth/token.ts`; test.

**Flow:**
- POST form: `grant_type=authorization_code` → `consumeCode(code, code_verifier)` → `mintAccessToken(claims)` + refresh token (`storeRefreshToken`) → JSON `{access_token, token_type:"Bearer", expires_in, refresh_token}`.
- `grant_type=refresh_token` → `rotateRefreshToken(refresh_token)` → new access + new refresh.

- [ ] **Step 1:** Failing test: full happy path (mint code via flow store, exchange, verify the returned access token via the public JWKS); refresh path returns a new pair and invalidates the old refresh.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement `token.ts`; add `vercel.json` route.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit: `feat(oauth): token endpoint (authorization_code + refresh_token)`.

### Task 2.9: Mock-IdP end-to-end (full flow)

**Files:** `packages/temper-cloud/src/oauth/e2e.saml.test.ts` (Vitest integration) using a self-signed IdP keypair fixture to synthesize a signed `SAMLResponse`.

- [ ] **Step 1:** Failing test: seed a `kb_saml_idp` row (IdP cert = fixture pub), call `authorize`→capture relayState, synthesize a signed assertion for a persistent NameID + email attr, POST to `acs`→capture code, POST to `token` with the PKCE verifier→get access token, verify it with the public JWKS. Assert the claims (`sub` = NameID, `email`, `email_verified:true`).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Fill any glue gaps surfaced.
- [ ] **Step 4:** Run → PASS. (This is the functional-login proof the PR needs.)
- [ ] **Step 5:** Commit: `test(oauth): full mock-IdP SAML → code → token e2e`.

---

## Milestone M3 — Cross-stack e2e: SAML-minted token → `temper-api`

### Task 3.1: E2E — AS-minted token authenticates against `temper-api`

**Files:** extend `tests/e2e/tests/eddsa_auth_test.rs` (or a new `saml_flow_test.rs`) — reuse the mjs mint path from Task 1.4, but drive the claims produced by `mapProfileToClaims` (sub = persistent NameID, email, email_verified).

- [ ] **Step 1:** Failing test: a token carrying `{sub:"saml-persistent-id", email:"a@corp.io", email_verified:true, iss, aud}` minted by the TS path → `/api/profile` 200 AND a new `kb_profiles` row + `kb_profile_auth_links` row is created (query the pool) with `auth_provider` = the instance's `AUTH_PROVIDER_NAME`.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Wire the test (set `setup_eddsa`'s `ApiConfig.auth_provider_name = "saml:test-idp"` to assert the namespacing).
- [ ] **Step 4:** Run → PASS. Confirms the identity contract: SAML → AS → `resolve_from_claims` JIT.
- [ ] **Step 5:** Commit: `test(e2e): SAML-shaped AS token drives profile JIT in temper-api`.

---

## Milestone M4 — Surface repoint + operator docs

### Task 4.1: CLI provider preset for a SAML/AS instance

**Files:** Modify `crates/temper-cli/src/commands/init.rs` (near the `Idp` enum `:35` and the provider-writing logic `:532`); test in the same file's test module.

**Interfaces:**
- Produces: a way for `temper init` to write a `[[auth.providers]]` entry with `authorize_url = "{base}/oauth/authorize"`, `token_url = "{base}/oauth/token"`, `callback_url = "{base}/api/auth/cli-callback"`, `audience = "{api}/api"`, `scopes = ["openid","offline_access"]`, `client_id = "temper-cli"`, and `provider = "temper-as"`.

- [ ] **Step 1:** Failing test asserting the written config points `authorize_url`/`token_url` at the AS endpoints (not Auth0) for the new preset.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement the preset (do not change compiled-in defaults; this is an additive preset). No change to `login.rs` — the existing PKCE+loopback+refresh flow is issuer-agnostic.
- [ ] **Step 4:** Run `cargo nextest run -p temper-cli init`; `cargo make check`.
- [ ] **Step 5:** Commit: `feat(cli): temper-as provider preset for SAML instances`.

### Task 4.2: UI login repoint (config/env)

**Files:** `packages/temper-ui/.env.example` (document `OIDC_*` pointing at the Temper AS + its RFC 8414 metadata); verify `oidc-core.ts` `resolveOidcConfig` already consumes `OIDC_ISSUER`/`OIDC_CLIENT_ID`; add a note that the AS metadata must expose `authorization_endpoint`/`token_endpoint`/`jwks_uri` (Task 2.6 does).

- [ ] **Step 1:** Failing test in `oidc-core.test.ts`: `resolveOidcConfig({OIDC_ISSUER:"https://inst/oauth", OIDC_CLIENT_ID:"temper-ui"})` yields a config whose discovery would hit the AS metadata. (If discovery expects `/.well-known/openid-configuration`, add handling to also accept the AS's `/.well-known/oauth-authorization-server` — small `parseDiscovery` extension; test it.)
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement the minimal `parseDiscovery`/config change so the UI can discover the AS. `session.ts`/`writeSession` unchanged.
- [ ] **Step 4:** Run `cd packages/temper-ui && bun run check`.
- [ ] **Step 5:** Commit: `feat(ui): allow OIDC client config to target the Temper AS`.

### Task 4.3: Operator documentation

**Files:** Create `docs/guides/self-hosting-saml.md`; modify `docs/guides/self-hosting-okta.md`.

- [ ] **Step 1:** Write `self-hosting-saml.md`: architecture summary; how to register the SP with the IdP (ACS URL = `{instance}/oauth/saml/acs`, SP entityID, NameID persistent, email + stable-id attribute statements); the `kb_saml_idp` row to insert; the AS env vars (`AS_SIGNING_KEY_PKCS8`, `AS_SIGNING_KID`, `AS_ISSUER`, `AS_AUDIENCE`); the `temper-api` env (`JWKS_URL={instance}/oauth/jwks`, `AUTH_ISSUER=AS_ISSUER`, `AUTH_AUDIENCE`, `AUTH_PROVIDER_NAME=saml:<idp-key>`); CLI + UI provider config; the honest limit (reconcile-on-login staleness; SCIM = Phase 3).
- [ ] **Step 2:** Add a cross-reference in `self-hosting-okta.md` "Not covered" section pointing to the new native-SAML option.
- [ ] **Step 3:** `markdownlint` / `cargo make check` docs step passes.
- [ ] **Step 4:** Commit: `docs(guides): self-hosting SAML SP guide`.

---

## Self-Review

**Spec coverage** (each spec §):
- §3/§4.1 AS in temper-cloud → M1/M2 (all `api/oauth/*`). ✓
- §4.2 three surfaces, UI+CLI ship → M4.1 (CLI), M4.2 (UI); MCP explicitly deferred. ✓
- §4.3 identity contract (persistent NameID/attr, email, email_verified=true, provider via config) → 2.3 + 3.1. ✓
- §4.4 EdDSA signing, JWKS, TTL, stored refresh → 1.2/1.3/2.5/2.8. ✓
- §5 SAML SP (SP-initiated, checklist, node-saml, endpoints) → 2.4/2.7. ✓ (IdP-initiated disallowed: `authorize` is the only entry; ACS requires a matching pending `relayState`.)
- §6 data model (kb_saml_idp + AS state tables, conventions) → 2.1. ✓
- §7 temper-api EdDSA allow-list + e2e → M0. ✓
- §8 testing (SP validation, round-trip, mock-IdP e2e, CLI) → 0.2/1.4/2.4/2.9/3.1/4.1. ✓
- §9 deferred (mixed-mode, Phase 2/3, multi-IdP, MCP wiring) → not in plan by design. ✓

**Placeholder scan:** the canned-assertion + IdP fixtures (Tasks 2.4/2.9) are generated by explicit fixture scripts, not left as "TODO". SAML library assumed `@node-saml/node-saml` (Task 1.0); if the impl spike rejects it, Tasks 2.4/2.7 adjust to `samlify`'s API — noted, not hidden.

**Type consistency:** `MintedClaims {sub,email,email_verified}` used identically across `mint.ts` (1.3), `mapProfileToClaims` (2.3), `flow.ts` (2.5), `token.ts` (2.8). `relayState`/`code_hash` names consistent across `flow.ts` and the endpoints. `getPublicJwks`/`getSigningKey` consistent across 1.2/1.3/2.6.

**Staging note:** M0 and M1 are shippable early; M2 is the large functional-login block (build across sessions on this branch); M3 proves the seam; M4 wires surfaces + docs. Per decision, the merged PR is a functional SAML login (M0–M3 minimum; M4 completes operability).
