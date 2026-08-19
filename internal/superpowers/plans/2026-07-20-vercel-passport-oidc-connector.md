# Implementation plan — OIDC upstream connector + typed deployment posture (Option B1)

Companion to the research/design doc
[`docs/superpowers/specs/2026-07-20-vercel-passport-native-integration-research.md`](../specs/2026-07-20-vercel-passport-native-integration-research.md).

**Goal.** Add an `oidc` upstream federation connector behind Temper's existing Authorization Server,
as a sibling of the SAML SP, and make an instance's auth posture first-class and typed. Target
posture: **`TemperAs` + `oidc`(corporate IdP)**, with Vercel Passport optionally gating the separate
`temper-ui` project. B1 is additive: `ExternalIdp`/Auth0 (temperkb.io) and `saml`-direct (Okta) stay
untouched and viable.

**Non-goals.** No change to the Rust JWT-validation identity (issuer/JWKS/audience stay env,
boot-blocking). No multi-issuer resource server. No Vercel-account identity (that was B2, declined).
No change to `client_credentials` machine principals.

**Decided (see spec Part 6).** Two connector types only: `saml`, `oidc`. "Vercel OIDC" = the `oidc`
connector pointed at Vercel, not a distinct type. Crypto identity stays in env. Typed
`kb_auth_connector`, not a KV settings blob.

---

## Architecture recap (what already exists and is reused verbatim)

The federation building blocks in `packages/temper-cloud/src/oauth/` are IdP-agnostic and are reused
with **no change**:
- `createPendingFlow` / `bindCodeToFlow` / `consumeCode` (`flow.ts`) — PKCE pending-flow + code binding.
- `mintAccessToken` / refresh rotation (`mint.ts`, `flow.ts`) — Temper-signed EdDSA JWT + refresh.
- `handleToken` (`endpoints.ts`) — the `/oauth/token` grants (authorization_code, refresh_token,
  client_credentials). **Untouched.**
- `reconcileMemberships` (`oauth/reconcile.ts`) — IdP-driven team memberships.
- RFC 8414/9728 discovery + DCR proxy (`oauth/metadata.ts`, `crates/temper-mcp/src/discovery.rs`).

Only two things are SAML-specific and get an `oidc` sibling: the **login redirect** and the
**upstream-response → `MintedClaims`** mapping. Everything downstream of `{sub, email}` is shared.

`MintedClaims` (the contract a connector must produce) is unchanged:
```ts
interface MintedClaims { sub: string; email: string; email_verified: boolean; }
```

---

## Phase 0 — Schema: typed `kb_auth_connector` (head + per-type detail)

A discriminated union in SQL, done the typed way: a thin **head** table carrying the discriminant +
activation, and **per-type detail** tables so each type's config stays typed and non-nullable in its
own table. This realizes the spec's "generalize `kb_saml_idp` → discriminated `kb_auth_connector`"
without a wide nullable table or a JSON blob.

- `kb_auth_connector` (head): `id`, `connector_key` (stable slug), `connector_type` enum
  (`'saml' | 'oidc'`), `is_active bool`, timestamps. Exactly one active row (partial unique index on
  `is_active WHERE is_active`).
- `kb_saml_idp` (existing, detail): keyed 1:1 to a `kb_auth_connector` row of type `saml`. Columns
  unchanged; add the FK. Existing single-active-row semantics move up to the head table.
- `kb_oidc_idp` (new, detail): keyed 1:1 to a `kb_auth_connector` row of type `oidc`. Typed,
  **non-secret** columns: `issuer`, `authorization_endpoint`, `token_endpoint`, `jwks_uri`,
  `userinfo_endpoint` (nullable), `client_id`, `client_secret_ref` (an **env var *name***, not the
  secret — see Security), `scopes` (text[], default `{openid,email,profile}`), `sub_claim`
  (default `sub`), `email_claim` (default `email`), `groups_claim` (nullable). No secret column.

**Migration posture.** Per the repo's additive-only-on-`main` invariant, the `main` migration is
purely additive (new tables/enum, nullable FK on `kb_saml_idp`). Back-filling the existing SAML row
into a `kb_auth_connector` head row + flipping reads to the head table is an **operator-run cutover**
per target (temperkb.io has no SAML row, so it's a no-op there). Document in the cutover runbook;
do not big-bang on `main`.

**Loader** (`saml/config.ts` → generalize to `oauth/connector.ts`): `loadActiveConnector(db)` returns
a discriminated `{ type: 'saml', idp: SamlIdpRow } | { type: 'oidc', idp: OidcIdpRow }`. `loadActiveIdp`
stays as a thin `saml`-typed accessor for the unchanged SAML paths.

---

## Phase 1 — The `oidc` connector legs (temper-cloud, TypeScript)

Siblings of `/oauth/saml/{login,acs}` in `oauth/endpoints.ts`, new module `oauth/oidc.ts`:

1. **`GET /oauth/oidc/login?rs=<relayState>`** — load the active `oidc` connector; generate an
   **upstream** PKCE pair + nonce; stash `{ upstream_code_verifier, nonce }` keyed by `rs` (extend
   the pending-flow row, or a sibling `kb_oidc_login_state` keyed by relay state); 302 to the
   upstream `authorization_endpoint` with `response_type=code`, `scope`, `state=rs`,
   `code_challenge` (S256), `nonce`, `redirect_uri = {base}/oauth/oidc/callback`.
2. **`GET /oauth/oidc/callback?code=&state=`** — validate `state` (= relay state, one-time); exchange
   `code` at the upstream `token_endpoint` (with the upstream `code_verifier`, `client_secret` from
   the ref); **verify the `id_token`**: signature vs `jwks_uri` (jose `createRemoteJWKSet`), `iss`,
   `aud == client_id`, `exp`, and `nonce` match; optionally call `userinfo_endpoint`; map to
   `MintedClaims` via `mapOidcClaimsToMinted` (mirror of `mapProfileToClaims`); then reuse
   `bindCodeToFlow` and 302 back to the client's `redirect_uri` with Temper's `?code` + `state`.
3. **`mapOidcClaimsToMinted(claims, connector)`** (`oauth/oidc.ts`, pure): `sub = claims[sub_claim]`,
   `email = claims[email_claim]`, `email_verified` from the upstream claim (default true only if the
   IdP asserts it — OIDC carries `email_verified`, unlike a SAML assertion where we infer it). Throw
   with a non-PII message if `sub`/`email` absent.
4. **`handleAuthorize` dispatch** (`oauth/endpoints.ts`): after stashing the pending flow, branch on
   `loadActiveConnector().type` → 302 to `/oauth/saml/login` or `/oauth/oidc/login`. No other change
   to the authorize/token surface.

---

## Phase 2 — IdP-driven memberships for OIDC

- `extractOidcGroups(claims, connector)` mirroring `extractGroups`: `null` when `groups_claim` unset
  or absent (signal-missing guard preserved), `[]`/array when present.
- In the callback, reuse `reconcileMemberships` with `provider: "oidc:<connector_key>"`,
  `external_user_id: sub`, `email`, `email_verified`, `groups`. Same fail-open posture as SAML ACS
  (a reconcile error must never block login).

---

## Phase 3 — Declared deployment posture + boot cross-check

Make the posture explicit and fail-closed, mirroring `auth_config.rs`'s philosophy.

- Enumerated `deployment_mode`: `external_idp | saml_direct | oidc_direct` (Vercel Passport is
  `oidc_direct` + a UI gate; it is not its own token mode). **Decided: an env var**, resolved in/next
  to `parse_auth_config` — consistent with `auth_config.rs` (set-once boot posture, never twiddled at
  runtime), not a DB row.
- **Boot cross-check** (extend `parse_auth_config` or a sibling): assert the declared mode agrees
  with the env posture and the active connector — e.g. `external_idp` ⇒ `AS_ISSUER` unset;
  `saml_direct`/`oidc_direct` ⇒ `AS_ISSUER` set **and** an active connector of the matching type
  exists. Refuse boot (named error, no values printed) on disagreement.

---

## Phase 4 — Discovery / metadata (mostly unchanged)

- `oauth/metadata.ts` already advertises the Temper AS endpoints in `AS_ISSUER` mode — **no change**;
  MCP clients keep seeing the AS regardless of upstream connector.
- SP metadata endpoint (`/oauth/saml/metadata`) stays SAML-only. OIDC has no equivalent "SP metadata"
  document; instead the connector's `redirect_uri` (`/oauth/oidc/callback`) must be registered at the
  upstream IdP (operator step, documented in the runbook).

---

## Phase 5 — Vercel Passport on `temper-ui` (optional, parallel, additive)

Separate Vercel project, no coupling to the above. Enable Passport pointed at the same corporate IdP;
if the UI's server code needs the identity, read `x-vercel-oidc-passport-token`, verify vs Vercel's
JWKS, and trust the `external_sub` claim. The API/MCP project stays ungated and keeps validating
Temper-issued JWTs.

---

## Security checklist

- **Upstream leg uses its own PKCE + `nonce`**; `state` = one-time relay state, bound and consumed.
- **`id_token` fully verified**: signature (JWKS), `iss`, `aud == client_id`, `exp`/`nbf`, `nonce`.
- **`client_secret` lives in env, never in the DB.** It is one deployment-level secret (like the
  `AS_*` family / the EdDSA signing keys), not a per-user grant — so it does **not** use the Slack
  vault (that pattern exists for many user-scoped secrets that need at-rest DB encryption). `kb_oidc_idp`
  stores only `client_secret_ref` (the env var *name*); the value is set alongside `AS_*`, e.g.
  `OIDC_UPSTREAM_CLIENT_SECRET`. **Distinct prefix from `AS_*` deliberately:** `AS_*` is *our* AS's
  identity (the issuer we mint); the upstream client credentials are the party we authenticate *to* —
  keep the two identities visibly separate, per the `auth_config.rs` anti-conflation discipline. A
  confidential OIDC client needs the live secret to call the token endpoint, so it cannot be hashed;
  keeping it in env (not DB) is what keeps it out of the connector table.
- **Crypto identity unchanged**: `AUTH_ISSUER`/`JWKS_URL`/`AUTH_AUDIENCE`/`AS_*` stay env,
  boot-blocking; the connector never touches them.
- **No PII in logs**: callback/token-exchange errors log message only (same rule as SAML ACS).
- **Reconcile fail-open**: never let a membership-reconcile error reject a valid login.

## Testing

- `temper-cloud` vitest: the two `oidc` legs against a mock upstream AS (issue a signed id_token from
  a test JWKS); `mapOidcClaimsToMinted` + `extractOidcGroups` pure-unit; `handleAuthorize` dispatch
  by connector type; boot cross-check accept/reject matrix.
- Existing `flow.ts`/`mint.ts`/SAML tests unchanged and must stay green (proves reuse didn't regress).
- SQL: `kb_oidc_idp` + head-table queries use `sqlx`-macro'd or vitest-DB paths per repo convention;
  regenerate caches if Rust reads the new tables (only if Phase 3 reads them from Rust).

## Rollout order

Phase 0 (additive migration) → Phase 1 (legs) → Phase 2 (memberships) → Phase 3 (posture gate) →
Phase 4 (docs) can land in one or two PRs (0–2 together, 3 optionally separate). Phase 5 is
independent and can land anytime. Operator cutover (back-fill SAML into the head table, set the
active connector, register the OIDC redirect URI, set the declared mode) is per-target and runbooked,
not on `main`.
