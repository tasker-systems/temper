# SAML SP via a Minimal Temper Authorization Server — Design (Phase 0 + 1)

**Date:** 2026-07-01
**Status:** Design — approved for planning
**Issue:** [#224 — SAML SP with profile, role, and team provisioning](https://github.com/tasker-systems/temper/issues/224)
**Scope:** Phase 0 (decisions) + Phase 1 (SAML authentication only). Role/team provisioning (Phase 2) and SCIM (Phase 3) are named future phases, out of scope here.

---

## 1. Problem & motivation

Temper today authenticates exclusively with bearer JWTs from a single configured OIDC
issuer. `temper-api`'s trust boundary is "a validly-signed JWT from the configured
issuer," validated against a cached JWKS. The current enterprise story
(`docs/guides/self-hosting-okta.md`) punts SAML *upstream*: federate your SAML IdP into
an OIDC-capable broker (Okta) and let the broker mint the OIDC tokens Temper validates.
Temper "is not a SAML service provider."

A growing class of enterprise self-hosters standardize on **SAML** and expect the
application to be a first-class **SAML Service Provider (SP)** — not to require an
OIDC broker in front of it. This design makes Temper a native SAML SP.

Because Temper is only whole as **CLI + API + MCP**, the design's controlling
constraint is that the SAML approach must serve all three surfaces eventually, even
though Phase 1 ships only two of them. The chosen architecture is explicitly
forward-compatible with MCP rather than a UI-only bolt-on.

## 2. Grounding — verified current state

Verified against the code at this checkout (file:line):

- **Bearer-JWT middleware, single issuer.** `crates/temper-api/src/middleware/auth.rs`
  `require_auth` extracts the Bearer token, validates against the cached JWKS, and
  builds `AuthClaims` from `sub`/`email`/`email_verified`/`exp`/`iat`. Issuer/audience
  are **scalars**: `state.config.auth_issuer` (one `String`), `auth_audience`
  (`Option<String>`); `JwksKeyStore` (`crates/temper-services/src/state.rs`) holds a
  single `url`, and `validation()` calls `set_issuer(&[issuer])` with exactly one
  issuer. `refresh()` grabs the first usable key from one JWKS with **no `kid`
  matching**.
- **Provider is config, not code.** `crates/temper-services/src/config.rs` `ApiConfig`
  reads `JWKS_URL`/`AUTH_ISSUER`/`AUTH_AUDIENCE`/`AUTH_PROVIDER_NAME`.
  `AUTH_PROVIDER_NAME` is only a label + DB lookup/cache key; it does **not** switch
  validation logic. `crates/temper-core/src/types/auth.rs` `AuthProvider` is a plain
  **struct** with a free-form `name: String` (the `okta`/`auth0`/`neon_auth` values are
  doc examples, not enum variants) — so a new `saml:<idp>` provider needs no type change.
- **Profile JIT already exists; no role/team at login.**
  `crates/temper-services/src/services/profile_service.rs` `resolve_from_claims`:
  (a) looks up `kb_profile_auth_links` by `(auth_provider, auth_provider_user_id)`;
  (b) reconciles onto an existing profile by email **only when
  `email_verified == Some(true)`**; (c) else creates a new profile + default auth link
  + per-surface emitter entities + a `default` context. It assigns **no team or role**.
- **Teams are temper-owned.** `crates/temper-core/src/types/team.rs`: "Teams are fully
  owned by temper, not delegated to the auth provider." `TeamRole` is a fixed enum
  `Owner > Maintainer > Member > Watcher`. Auto-join provisioning is **Postgres
  functions** (`ensure_auto_join_memberships`, `backfill_auto_join_team` in the
  auto-join migration), not Rust. `kb_team_members` has **no provenance column**.
- **The CLI already speaks auth-code + PKCE.** `crates/temper-client/src/login.rs` runs
  OAuth2 Authorization Code + PKCE (S256) with a `127.0.0.1:0` loopback callback,
  relayed via a hosted `callback_url`; token exchange at `token_url`; refresh via the
  `refresh_token` grant (`offline_access`). Tokens cache to `~/.config/temper/auth.json`
  as a `StoredAuth` struct (chmod 0600). Issuer selection is **client-side config**
  (`[[auth.providers]]`: `authorize_url`/`token_url`/`client_id`/`audience`/
  `callback_url`/`scopes`) — nothing Auth0-specific is compiled in.
- **The UI already mints sessions.** `packages/temper-ui/src/lib/server/oidc.ts`
  (generic OIDC auth-code + PKCE via discovery), `session.ts` (encrypted JWE session
  cookie holding the access token; the Profile is *not* in the cookie), `proxy.ts` /
  `hooks.server.ts` (same-origin reverse proxy of `/api`, `/mcp`, `/oauth`,
  `/.well-known` to `API_BASE_URL`).
- **Temper signs nothing today.** No signing key, no published JWKS, no `/jwks` route
  anywhere. `temper-mcp/src/discovery.rs` publishes RFC 8414/9728 metadata but advertises
  the **Auth0 tenant** as issuer. Making Temper an issuer is greenfield on the signing
  side.
- **No existing SAML code.** Zero SAML dependencies; the only hit is a CLI test that
  asserts `saml` is rejected as an unknown `--idp`.

## 3. Guiding principle

> **Bridge SAML to the existing JWT trust boundary; don't fork it.** Introduce a
> minimal **Temper Authorization Server (AS)** whose *upstream authentication method*
> is SAML and whose *output* is a short-lived Temper-signed JWT carrying the existing
> `AuthClaims` shape. `temper-api` becomes a resource server that trusts the AS as its
> issuer. The auth **protocol** (SAML, browser-facing, TypeScript on Vercel) is fully
> decoupled from the **credential** every surface shares (a JWT).

This preserves "provider is config, not code," reuses `resolve_from_claims` verbatim,
and means the CLI never has to speak SAML.

## 4. Architecture

### 4.1 Components

```
                         ┌───────────────────────────────────────────┐
                         │  Temper Authorization Server (TS, Vercel)   │
   SAML IdP  ──assertion─▶│  co-hosted in the SvelteKit UI server       │
   (Okta, …)   (browser)  │                                             │
                         │  SAML SP:   /auth/saml/{login,acs,metadata} │
                         │  OAuth AS:  /oauth/authorize (code+PKCE)     │
                         │             /oauth/token   (+ refresh)       │
                         │             /.well-known/oauth-authorization-server (RFC 8414)
                         │             JWKS endpoint                     │
                         │  Signing:   Ed25519 key (Vercel env, kid)     │
                         └───────────────┬─────────────────────────────┘
                                         │ mints Temper JWT (AuthClaims shape)
             ┌───────────────────────────┼───────────────────────────┐
             ▼                           ▼                           ▼
        UI (session)              CLI (code+PKCE loopback)      MCP (code+PKCE)
        JWE cookie                ~/.config/temper/auth.json    [designed, not shipped]
             │                           │                           │
             └───────────── Bearer Temper JWT ─────────────────────┘
                                         ▼
                          temper-api (resource server, unchanged)
                          require_auth → resolve_from_claims
```

The AS is **co-hosted in the SvelteKit UI server** (it already runs OIDC, mints the
session, and proxies `/oauth` + `/.well-known` same-origin). It is not a separate
deployment.

### 4.2 One grant, three surfaces

All surfaces are **auth-code + PKCE** clients of the AS. This is the key simplification:
the CLI already uses exactly this grant, and MCP uses it too, so the AS needs only one
authorization grant (plus refresh).

- **UI** — co-hosts the AS. On its own login the SvelteKit server runs SAML
  (`/auth/saml/login` → IdP → `/auth/saml/acs`), validates the assertion, mints the
  Temper JWT, and writes the **same encrypted JWE session cookie** `session.ts` writes
  today. No self-redirect through `/oauth/authorize` — for its own session it mints
  in-process. The ACS distinguishes a UI-session initiation from an external OAuth-client
  authorize request via **RelayState**.
- **CLI** — **config repoint only, no code change.** A `[[auth.providers]]` entry whose
  `authorize_url`/`token_url` point at the AS; the existing PKCE + loopback + refresh
  flow in `login.rs` works verbatim. On a SAML instance, hitting `/oauth/authorize`
  triggers the SAML dance in the browser; after ACS the AS redirects back to the CLI's
  loopback with an authorization code, which the CLI exchanges at `/oauth/token`. The
  hosted callback-relay endpoint (analogous to today's `…/api/auth/cli-callback`) must
  exist on the instance's own host.
- **MCP** — **designed, not shipped.** Same grant. `temper-mcp`'s existing RFC 8414/9728
  discovery endpoints repoint from the Auth0 tenant to the AS in a later increment.
  Known MCP-phase needs, anticipated by the foundation but not built now: **RFC 7591**
  dynamic client registration (MCP clients are third parties) and **RFC 9728**
  resource-indicator / audience targeting (so a token can target the `api` vs `mcp`
  resource).

### 4.3 The Temper token (identity contract)

The AS mints a JWT carrying the existing `AuthClaims` shape, so `resolve_from_claims`
is consumed unchanged:

| Claim | Source |
|-------|--------|
| `provider` | `"saml:<idp-key>"` — namespaced so links never collide with the OIDC `okta`/`auth0` providers |
| `sub` / `external_user_id` | **persistent-format NameID** when present; else an **operator-configured stable-id attribute** (e.g. the Okta user id); **never email** |
| `email` | the email attribute / email-format NameID (SAML has no OIDC `/userinfo`, so the SP supplies this directly) |
| `email_verified` | `true` — a signature-checked assertion from the trusted IdP *is* the verification |

`email_verified = true` means a SAML login **reconciles onto an existing profile** with
the same email (created via OIDC, or a prior SAML login) rather than duplicating it —
the "teams survive provider swaps" property at the identity layer. Choosing a stable,
non-email `sub` is what prevents duplicate profiles when a user's email changes at the
IdP.

### 4.4 Signing & token lifetime

- **Ed25519 (EdDSA)** signing key — consistent with what `temper-api` already validates.
  Held in Vercel env, tagged with a `kid`, published at the AS's JWKS endpoint. Cert/key
  rotation is supported by serving multiple keys keyed by `kid`.
- **Short-lived access tokens** (~15 min, tunable) + a **refresh token**. Refresh is
  non-negotiable: the CLI relies on the `refresh_token` grant today; without it every
  expiry forces a browser round-trip. Refresh becomes a Temper-side concern (there is no
  OIDC refresh token in the SAML path).
- Never place assertion contents or the minted token in URL parameters.

## 5. SAML SP details

- **SP-initiated only in v1.** IdP-initiated flows lack SP-generated RelayState/CSRF
  protection; they are disallowed and the reduced-guarantee rationale is documented.
- **ACS validation checklist** (all mandatory unless noted):
  - Verify the assertion **signature** against the configured IdP certificate; reject
    unsigned or wrongly-signed assertions. Support IdP cert rotation.
  - `Audience` (`AudienceRestriction`) **==** the SP Entity ID, exactly.
  - `NotBefore` / `NotOnOrAfter` enforced with **bounded clock skew** (~5 min).
  - `Recipient` / `Destination` **==** the ACS URL.
  - **Replay protection** on assertion IDs (one-time-use within the validity window).
  - Encrypted-assertion support is **optional** in v1; signature verification is
    mandatory regardless.
- **Toolkit:** a maintained TS SAML library — `@node-saml/node-saml` or `samlify`. Final
  selection is a plan-time decision (evaluate maintenance, encrypted-assertion support,
  and API fit); do not hand-roll XML canonicalization or signature handling.
- **Endpoints:** `/auth/saml/login` (SP-initiated AuthnRequest), `/auth/saml/acs`
  (receive + validate), `/auth/saml/metadata` (SP metadata for the operator to hand the
  IdP).

## 6. Data model & configuration

- **New additive migration** — a `kb_saml_idp`-style table keyed by `idp_key` holding:
  IdP signing certificate(s), IdP SSO URL, IdP entityID/issuer, NameID format
  preference, and the attribute mapping (email attribute, stable-id fallback attribute).
  The **non-singleton keyed shape** admits a second IdP additively even though v1 runs a
  single active IdP per instance. Cert rotation is a **data update**, not a redeploy. The
  TS SP reads it via the Neon serverless client. Additive-only — safe under the
  `main`-auto-deploy invariant.
- **`temper-api` config:** a SAML instance sets `JWKS_URL`/`AUTH_ISSUER`/`AUTH_AUDIENCE`
  to the AS. **No Rust code change** — single-issuer validation stays. temperkb.io keeps
  trusting Auth0 alone, untouched.
- **CLI onboarding:** an operator writes (or `temper init` gains a preset that writes) a
  `[[auth.providers]]` entry pointing at the AS. No compiled-in defaults change.

## 7. `temper-api` impact

Essentially none. The instance trusts the AS as its **single** issuer. Because the AS
mints EdDSA JWTs with the `AuthClaims` shape, `require_auth` validates them exactly as it
validates Auth0 tokens today, and `resolve_from_claims` performs profile JIT unchanged.
The single→multi issuer rework (issuer sets + `kid` matching + `refresh()` rewrite) is
**deferred** to the mixed-mode phase (§9).

## 8. Testing

- **SP validation (unit):** canned assertions — valid, tampered-signature, expired
  (`NotOnOrAfter`), not-yet-valid (`NotBefore`), wrong-audience, wrong-recipient,
  replayed-ID — each asserting accept/reject.
- **Token round-trip (unit):** mint an EdDSA Temper JWT, publish the JWKS, verify it with
  the same validation path `temper-api` uses; assert the `AuthClaims` mapping.
- **E2E:** a mock-IdP signed assertion → AS ACS → minted Temper JWT → `temper-api`
  `require_auth` → `resolve_from_claims` creates/reconciles a profile. Covers the full
  seam without a live IdP.
- **CLI (integration):** point a `[[auth.providers]]` entry at a stub AS and confirm the
  existing `login.rs` PKCE+loopback+refresh flow obtains and caches a token unchanged.

## 9. Out of scope

### Rejected (load-bearing decisions — resist scope creep)

- **Native Rust ACS in `temper-api`.** The Rust SAML ecosystem is thin; it would mean
  owning XML canonicalization + signature validation and duplicating the session/cookie
  machinery the UI already has. SAML is browser-facing and belongs where sessions are
  minted.
- **Email-as-NameID.** Breaks on IdP email changes → duplicate profiles.
- **Auto-creating a Temper team per IdP group.** Violates the temper-owned-teams
  principle, invites slug collisions, and over-delegates structure to the IdP. (Relevant
  to Phase 2; recorded here to bound it.)

### Deferred (in scope for a later phase)

- **Mixed-mode multi-issuer** — one instance trusting Auth0 *and* the Temper AS
  simultaneously. Requires reworking `temper-api` to issuer **sets** + per-issuer JWKS +
  `kid` matching + `refresh()` rewrite. Not needed for a pure-SAML instance.
- **Phase 2 — role + team provisioning (JIT reconcile-on-login).** Group→(team, role)
  mapping config; a **membership provenance** column (`source = 'idp' | 'native'`) so
  reconcile-on-login never clobbers temper-native memberships; optional admin-via-group;
  reconciliation of the first-admin bootstrap tension with the org-provisioning work.
  Its own spec.
- **Phase 3 — SCIM lifecycle.** SCIM 2.0 create/update/**deactivate** + Group Push for
  instances requiring immediate deprovisioning. JIT and SCIM are mutually exclusive per
  Okta connection; a stable shared identifier avoids duplicate accounts. Its own spec.
- **Multi-IdP behind one ACS**, and **IdP-initiated flows**.
- **MCP wiring** — designed here (§4.2), built later.

## 10. The honest limit

SAML JIT is authentication, not lifecycle management. Reconcile-on-login (Phase 2) gives
*eventual* correctness that fires only at login: a user removed or deactivated at the IdP
retains Temper access until their session expires and their next login fails. Instances
with immediate-revocation / offboarding-audit requirements need **SCIM** (Phase 3). This
is stated up front so it is a deliberate, named phase rather than a silent gap.

## 11. Key decisions (resolved in brainstorming)

1. **Spec scope:** Phase 0 + 1 (authn only).
2. **SP location:** a minimal Temper AS co-hosted in the UI tier (TS/Vercel), SAML
   upstream, minted Temper JWT as a trusted issuer — not a native Rust ACS.
3. **Surfaces:** UI (session) + CLI (auth-code+PKCE config repoint, no code change) in
   Phase 1; MCP designed-not-shipped (drop-in later).
4. **NameID / `sub`:** persistent NameID → configured stable attribute → never email.
5. **`email_verified`:** `true` for a signed trusted-IdP assertion (reconcile by email).
6. **Multi-IdP:** single active IdP per instance, non-singleton config shape.
7. **Issuer trust:** single-issuer repoint (instance trusts the AS alone); mixed-mode
   multi-issuer deferred.
8. **IdP config store:** a keyed DB table (`kb_saml_idp`), not env vars.
9. **Signing:** Ed25519 (EdDSA), matching existing validation.
10. **SAML flow:** SP-initiated only in v1.
