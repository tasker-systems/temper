# Vercel Passport as a native identity source for Temper — research & options

Grounding research for the client ask: *"make Temper work with Vercel Passport natively rather
than through the current SAML mechanism as IdP."* This document does **not** change code — it maps
what Vercel Passport is, what Temper's auth actually does today, and the three ways they can meet,
with a recommendation. The priority surfaces are **CLI, API, and MCP** (headless / Bearer-token
clients), not just the web UI.

## TL;DR

- **"Vercel Passport" is not an IdP.** It is a *deployment-protection gate* that federates to
  *your* corporate IdP (Okta / Microsoft Entra / Auth0 / any OIDC authorization server) and, after
  a **browser** login, forwards a Vercel-signed identity JWT to your server in the
  `x-vercel-oidc-passport-token` header. It is a front door, not the thing behind the door.
- **The browser-cookie model is the whole problem for CLI/API/MCP.** A Bearer-token client cannot
  follow Passport's 302-to-IdP redirect. Passport's only non-browser escape hatch today is the
  `x-vercel-protection-bypass` **shared secret**, which carries *no user identity*. So Passport as a
  gate in front of Temper's API/MCP would break exactly the surfaces the client cares about.
- **Temper is already built for this.** Temper is **its own OAuth 2.0 Authorization Server + SAML
  Service Provider** (in `packages/temper-cloud`), and the Rust API/MCP are **pure OIDC resource
  servers** that validate one configurable issuer/JWKS/audience. SAML is federated *upstream* behind
  Temper's own AS and re-minted as Temper-signed JWTs. **A Vercel/OIDC connector slots into the exact
  same seam SAML uses.**
- **Recommendation (decided): Option B1 — add the *corporate IdP* as an upstream OIDC connector
  behind Temper's existing Authorization Server**, mirroring the SAML SP. Nothing in the Rust
  validation path changes; the CLI's `authorization_code`+PKCE flow, the MCP DCR discovery, refresh
  tokens, and machine tokens all keep working unchanged. This is precisely the *"OAuth-native flow
  through Vercel, with the org routing its Vercel connector as a federated pole from the primary
  IdP"* that the ask floated — and it is a small, well-bounded change because the federation building
  blocks are already IdP-agnostic. **B1 forecloses nothing:** temperkb.io stays on Auth0
  (`ExternalIdp`), and the existing SAML-direct flow stays fully viable for orgs that don't want
  Vercel as a front door.
- Passport-the-gate is still useful — but **layer it on the web UI only** (`temper-ui` is a separate
  Vercel project), never in front of the API/MCP project.

---

## Part 1 — What "Vercel Passport" actually is (and its three cousins)

The name collides across four distinct Vercel products. Getting the client the right one depends on
keeping them straight.

| Product | What it is | Relevance to Temper |
|---|---|---|
| **Vercel Passport** (`/docs/passport`) | Deployment-protection gate. Redirects browser visitors to *your* corporate IdP, validates, sets a per-deployment session cookie, and forwards a Vercel-signed identity JWT to your server. Enterprise, ~$100/project/mo, public beta. | The thing the client is on. **Browser-first.** Federates to Okta/Entra/Auth0. |
| **Sign in with Vercel** (`/docs/sign-in-with-vercel`) | Vercel itself acting as an OAuth 2.0 / OIDC **authorization server**: `/oauth/authorize`, `/login/oauth/token`, `/login/oauth/userinfo`, `/login/oauth/token/introspect`, PKCE, `offline_access`+refresh, JWKS at `vercel.com/.well-known/jwks`. Identifies **Vercel accounts**. | A standards-clean OIDC AS Temper *could* federate to. **Access tokens are opaque (`vca_…`), verified by introspection; only the `id_token` is a JWKS-verifiable JWT.** |
| **Agent Passport / the "connection layer for agent credentials"** | The agent-credential piece shipped alongside Vercel's `eve` framework ("Sign in with Google, but for agents"). Beta, alongside Passport. | The forward-looking answer for *agent* identity, but immature and browser/consent-oriented today. |
| **Vercel OIDC** (`getVercelOidcToken`, `oidc.vercel.com`) | Workload identity for Vercel Functions → cloud resources (AWS/Azure/GCP). | **Not user identity.** Irrelevant to this ask beyond name confusion. |

### How Passport works (the mechanics that matter)

1. **Config:** enable Passport per-project or team-wide; point it at your IdP's OIDC **authorization
   server** issuer — the docs are explicit that it must be the *authorization server*, not just the
   IdP domain (e.g. Okta's `https://your_okta_domain.okta.com/oauth2/default`).
2. **Browser flow:** visitor hits a protected deployment → Vercel 302s to your IdP → IdP
   authenticates → Vercel validates the response, sets a **per-deployment session cookie**, and
   forwards a **Vercel-signed JWT** to your server.
3. **The identity token:** header `x-vercel-oidc-passport-token`. Vercel **strips any
   client-supplied copy** and injects the verified token, so the server can trust the value.
   Verify it against the issuer's JWKS. The reliable user identifier is the **`external_sub`** claim
   (the subject your IdP returned); `sub`/`scope` also carry `owner`, `connector_id`, `external_sub`
   in a stable Vercel format.
4. **Non-browser (the crux):** deployment protection is bypassed for automation via
   `x-vercel-protection-bypass: $VERCEL_AUTOMATION_BYPASS_SECRET` — a **shared secret**, no user
   identity. There is no "present a Bearer JWT and get through the gate as *user X*" path today; the
   agent-credential connection layer (beta) is Vercel's answer and is not yet a drop-in for a
   headless CLI/MCP resource server.

**Consequence for Temper.** `api/axum.rs`, `api/mcp.rs`, and `api/internal.rs` are all functions on
**one** Vercel project sharing one route table (`vercel.json`). Enabling Passport (deployment
protection) on that project gates `api.…/`, `/mcp`, and the crons behind the browser gate — which
breaks every Bearer-token CLI/API/MCP client unless they carry the bypass secret (losing per-user
identity). So Passport-as-a-gate is **not** a viable primary mechanism for Temper's headless
surfaces. It is viable as defense-in-depth on the **web UI** (`packages/temper-ui` is a *separate*
Vercel project — it can be gated independently while the API/MCP project stays ungated and keeps
validating Temper-issued OIDC JWTs).

---

## Part 2 — Temper's current auth architecture (the seam Passport plugs into)

The single most important finding: **Temper is not hardwired to Auth0.** An instance runs in one of
two modes, chosen purely by environment variables (`crates/temper-services/src/auth_config.rs`):

```rust
pub enum AuthMode {
    ExternalIdp,  // An external IdP (Auth0) mints tokens; temper is a pure resource server.
    TemperAs,     // Temper's own authorization server mints tokens.
}
```

- **`ExternalIdp`** — today's `temperkb.io`: Auth0 mints JWTs; the Rust API/MCP validate them.
- **`TemperAs`** — the enterprise/self-hosted shape: **Temper runs its own OAuth 2.0 Authorization
  Server** (`/oauth/authorize`, `/oauth/token`, `/oauth/jwks`, EdDSA-signed) and **federates an
  upstream SAML IdP**. The Rust surfaces only ever see **Temper-issued** JWTs.

### The Rust surfaces are pure OIDC resource servers

Both API and MCP share one validator, `JwksKeyStore` (`crates/temper-services/src/state.rs`):
fetch one JWKS URL, validate `iss` + `aud` + signature (RS256 for Auth0, EdDSA for the Temper AS).
Issuer/JWKS/audience come from env (`AUTH_ISSUER`, `JWKS_URL`, `AUTH_AUDIENCE`) via the one parser
`parse_auth_config` — **single issuer, single audience, by design** (it actively refuses divergent
`MCP_AUDIENCE`/`AS_AUDIENCE`). No Auth0 domain is hardcoded in the validation path.

### The SAML federation, which is the template to copy

In `TemperAs` mode, SAML is wrapped *inside* Temper's own authorize flow
(`packages/temper-cloud/src/oauth/endpoints.ts`):

```
GET /oauth/authorize   → validate PKCE, stash pending flow (createPendingFlow), 302 → /oauth/saml/login
GET /oauth/saml/login  → 302 → corporate SAML IdP SSO
POST /oauth/saml/acs   → validateAssertion → mapProfileToClaims → bindCodeToFlow → 302 back with ?code
POST /oauth/token      → consumeCode → mintAccessToken (Temper-signed EdDSA JWT) + refresh token
```

The claims a connector must produce are **tiny** (`packages/temper-cloud/src/oauth/mint.ts`):

```ts
export interface MintedClaims { sub: string; email: string; email_verified: boolean; }
```

Crucially, the federation building blocks — `createPendingFlow`, `bindCodeToFlow`, `consumeCode`,
`mintAccessToken`, refresh-token rotation, the DCR proxy, the RFC 8414/9728 discovery — are all
**IdP-agnostic**. Only two legs are SAML-specific: the *login redirect* and the
*assertion→claims* mapping (`mapProfileToClaims` in `saml/sp.ts`). Everything downstream of "we now
know `{sub, email}`" is shared.

### The other surfaces, for completeness

- **CLI** (`temper-client`): `authorization_code` + PKCE with a loopback callback (via a hosted
  relay), tokens cached to `~/.config/temper/auth.json`, `offline_access`/refresh supported. The IdP
  is fully pluggable through `[[auth.providers]]` in config (`authorize_url`, `token_url`,
  `client_id`, `audience`, `callback_url`, `scopes`) — **no code change** to point at a new AS.
  (Note: despite CLAUDE.md's "device authorization" wording, there is **no** RFC 8628 device flow;
  it's auth-code + PKCE.)
- **MCP** (`temper-mcp`): validates the same JWTs; serves RFC 9728 protected-resource metadata
  (`discovery.rs`) and a DCR proxy at `/oauth/register`; the RFC 8414 AS metadata is served by
  `temper-cloud` (`oauth/metadata.ts`) and **branches on `AS_ISSUER`** to advertise either the
  Temper AS's endpoints or Auth0's.
- **Machine principals** (`client_credentials`): registration-gated (`kb_machine_clients`,
  lookup-or-401), required claim `gty:"client-credentials"` + derivable `client_id`. The Temper AS
  already mints these itself (`mintMachineAccessToken`), claim-shaped to match Auth0 exactly.

---

## Part 3 — The three integration options

### Option A — Passport as an edge gate in front of Temper (deployment protection)

Enable Passport on the Temper Vercel project; do nothing to Temper's own auth.

- **Web UI:** works well — browser users get the corporate-IdP redirect and the deployment receives
  `x-vercel-oidc-passport-token`.
- **CLI/API/MCP:** ✗ **breaks.** Bearer-token clients cannot follow the redirect; the only way
  through is the `x-vercel-protection-bypass` shared secret, which erases per-user identity.
- **Verdict:** viable only as **defense-in-depth on the web UI project** (`temper-ui`), never in
  front of the API/MCP project. Not an answer for the priority surfaces.

### Option B — Vercel/corp-IdP as an upstream OIDC connector behind Temper's AS  ✅ recommended

Mirror the SAML SP with an OIDC connector: add `/oauth/oidc/login` + `/oauth/oidc/callback` (siblings
to `/oauth/saml/login` + `/oauth/saml/acs`) that federate to an upstream **OIDC authorization
server**, map the id_token/userinfo to `MintedClaims`, and re-mint a Temper JWT via the existing
`bindCodeToFlow` → `mintAccessToken` path.

Two sub-choices for *which* upstream — and this is the key decision for the client:

- **B1 — federate to the corporate IdP directly** (Okta / Entra / Auth0 — *the same issuer Passport
  itself points at*). This is the cleanest: Temper's AS becomes an OIDC client of the same corporate
  authorization server the org already runs, and Passport (on the web UI) and Temper's AS (for
  everything) share one source of truth. This is the literal reading of *"route the Vercel connector
  as a federated pole from the more primary IdP."*
- **B2 — federate to "Sign in with Vercel"** as the upstream OIDC AS. Standards-clean (PKCE,
  `offline_access`, id_token JWT verifiable at `vercel.com/.well-known/jwks`, `userinfo`). But it
  identifies **Vercel accounts**, not corporate identities, and its access tokens are opaque — so
  Temper would rely on the **id_token + userinfo** for `{sub, email}`. Fine mechanically; weaker as
  an enterprise identity source unless the org deliberately wants Vercel accounts to be the identity.

- **What changes:** only the two upstream legs in `temper-cloud` + config. **Nothing in the Rust
  validation path changes** — API/MCP keep validating the one Temper-AS issuer. CLI PKCE, MCP DCR,
  refresh, machine tokens: all unchanged.
- **Verdict:** smallest blast radius, preserves every surface, matches repo architecture exactly.

### Option C — Point Temper's resource servers directly at Vercel/corp issuer (`ExternalIdp` swap)

Skip the Temper AS; set `AUTH_ISSUER`/`JWKS_URL`/`AUTH_AUDIENCE` to the upstream's values (the Auth0
shape today, but pointed at Vercel or the corp IdP).

- Against the **corporate IdP directly** (Okta/Entra minting JWT access tokens with JWKS): ✓ works,
  and is essentially "make the corp IdP the direct issuer" — orthogonal to Passport. Loses Temper's
  owned refresh/rotation, its DCR proxy shape, and IdP-driven membership reconcile unless
  re-created.
- Against **"Sign in with Vercel":** ✗ awkward — its **access tokens are opaque** (`vca_…`), and the
  CLI/MCP send *access* tokens as Bearer. The Rust validator does JWKS verification, not
  introspection, so opaque tokens won't validate without new introspection code.
- Accepting **two issuers at once** (Auth0 *and* Vercel) is the invasive variant: it requires
  widening `AuthConfig.issuer/audience` to lists and multi-JWKS handling in `state.rs` — explicitly
  flagged as unfinished adversarial work in the auth audit.
- **Verdict:** acceptable only in the narrow B1-adjacent case (corp IdP mints JWT access tokens and
  the org is happy to drop Temper's AS). Otherwise more work and more loss than Option B.

---

## Part 4 — Recommendation

**Adopt Option B (B1 preferred): federate Vercel/the corporate IdP as an upstream OIDC connector
behind Temper's existing Authorization Server, and reserve Passport-the-gate for the web UI only.**

Why:

1. **It preserves every surface.** The whole point of the ask is *not* to retire functionality. B
   keeps CLI, API, MCP, refresh tokens, and machine principals working with zero Rust auth changes,
   because they all point at the Temper AS and the AS just swaps its upstream.
2. **It is the smallest change.** The federation plumbing is already IdP-agnostic; an OIDC connector
   is a sibling of the SAML SP with a *simpler* upstream leg (OIDC is easier than SAML XML/replay).
3. **It matches the mental model in the ask.** "OAuth-native flow through Vercel, org routes its
   Vercel connector as a federated pole from the primary IdP" ≈ Temper's AS federates to the corp
   OIDC AS (the same one Passport fronts). One source of truth, two consumers (Passport on web,
   Temper-AS everywhere).
4. **It keeps Passport where Passport is strong.** Passport is an excellent *browser* gate; put it on
   `temper-ui` for the human web experience and leave the headless surfaces to the OIDC/Bearer path.

---

## Part 4b — Deployment auth as a first-class, typed posture

The ask surfaced a real gap: an instance's auth posture is currently **inferred** from which env
vars and DB rows happen to be populated (`AS_ISSUER` present ⇒ AS mode; a single active
`kb_saml_idp` row ⇒ SAML upstream). Nowhere does an operator *declare* "this instance is deployment
mode X" and have boot *verify* the pieces agree. As more postures appear (community OAuth,
SAML-direct, Vercel-Passport-fronted, Vercel-OIDC), inference gets fragile. Make the posture
explicit and typed.

### Two orthogonal axes (this is the whole model)

The four "modes" in the ask are not four token pipelines — they are combinations of two axes:

**Axis 1 — token-issuance posture** (env, boot-blocking, the existing `AuthMode`, unchanged):
- `ExternalIdp` — an external OAuth/OIDC IdP mints tokens; Temper is a pure resource server.
  (Auth0/community, and **temperkb.io stays exactly here**.)
- `TemperAs` — Temper's own AS mints tokens, federating an upstream.

**Axis 2 — upstream federation connector** (only in `TemperAs`; DB-resident + typed, generalizing
today's `kb_saml_idp`):
- `saml` — direct to a managed SAML IdP (Okta). *Exactly today's flow — stays viable, unchanged.*
- `oidc` — direct to an OIDC authorization server. B1 points this at the **corporate IdP** (the same
  issuer Passport fronts). Pointed at `vercel.com` it is "Sign in with Vercel" (B2) — same code path,
  different config.

Mapping the ask's four deployment modes onto the axes:

| Deployment mode | Axis 1 | Axis 2 | Web-UI gate |
|---|---|---|---|
| Auth0 / community OAuth | `ExternalIdp` | — | none |
| SAML direct (Okta) | `TemperAs` | `saml` | none |
| **Vercel Passport (B1)** | `TemperAs` | `oidc` → corp IdP | **Passport on `temper-ui`** |
| Vercel OIDC / Sign in with Vercel | `TemperAs` | `oidc` → vercel.com | optional |

So **"Vercel Passport" is not a fourth token pipeline** — it is `TemperAs` + `oidc`(corp IdP) +
Passport enabled on the *separate* UI project. That is why B1 doesn't foreclose anything: an org that
doesn't want Vercel as a front door just uses `saml` (or `oidc` → their own IdP). Nothing is retired.

### The guardrail: identity stays in env, connector config goes in DB

Do **not** move the resource server's cryptographic identity (issuer / JWKS / audience) into a
mutable settings table. `auth_config.rs` is deliberately env-driven and boot-blocking for a reason it
documents at length: an ambiguous or runtime-mutable issuer/audience is a security hole (it once
silently disabled audience validation). Keep the boundary:

- **Env, boot-blocking, immutable per deploy** — Axis-1 posture + the crypto identity:
  `AUTH_ISSUER`, `JWKS_URL`, `AUTH_AUDIENCE`, `AS_ISSUER`/`AS_AUDIENCE`. Already correct; leave it.
- **DB, typed, operator-managed** — Axis-2 connector selection + its config (the `kb_saml_idp`
  precedent), plus non-crypto posture flags (e.g. "Passport expected on UI", org display name).

### On `kb_system_settings` — prefer a typed connector table, not a KV blob

A generic `kb_system_settings` key/value table invites exactly the untyped-JSON anti-pattern the
repo's code-quality rules forbid ("typed structs over inline JSON", "parse don't validate"). The
better shape, matching the existing precedent:

- Generalize `kb_saml_idp` → a typed **`kb_auth_connector`** table with a discriminated
  `connector_type ∈ {saml, oidc}` and typed, per-type config (SAML: cert / sso_url / entity ids /
  attr maps; OIDC: issuer / authorize / token / jwks / userinfo / client_id / secret-ref / scopes /
  claim map). One active connector per instance today; the discriminant leaves room for per-org
  connectors later.
- If a broader system-settings surface is genuinely wanted, keep **auth** out of a loose KV:
  represent the declared deployment mode as an explicit **enumerated value that boot cross-checks
  against env + the active connector** (fail-closed if they disagree), mirroring `auth_config.rs`'s
  "name the rule, verify it, refuse if violated" philosophy — not a switch that mutates behavior at
  runtime.

### Dispatch changes (small)

- `handleAuthorize` (`oauth/endpoints.ts`): dispatch to `/oauth/{saml,oidc}/login` by the active
  connector's `connector_type` instead of the hardcoded `/oauth/saml/login`.
- Add the `oidc` connector's two legs (`/oauth/oidc/login`, `/oauth/oidc/callback`) as siblings of
  the SAML pair; everything downstream of `{sub, email}` is shared.
- Reuse `reconcileMemberships` with `provider: "oidc:<connector_key>"` and the upstream `groups` claim.
- Rust validation path + discovery: unchanged (instance stays `TemperAs`; metadata already advertises
  the AS).

---

## Part 5 — Implementation sketch for Option B (for when we build it)

Not built here — this is the shape a follow-up PR would take.

1. **Upstream connector config** (new): store OIDC provider config analogous to `kb_saml_idp` — an
   `kb_oidc_idp` row or reuse the provider registry: `issuer`, `authorization_endpoint`,
   `token_endpoint`, `jwks_uri`/`userinfo_endpoint`, `client_id`, `client_secret`, `scopes`
   (`openid email profile`), claim map (`sub_claim`, `email_claim`, optional `groups_claim`).
2. **Two endpoints** in `packages/temper-cloud/src/oauth/` (siblings of the SAML pair):
   - `/oauth/oidc/login?rs=<relayState>` → build the upstream authorize URL (PKCE for the *upstream*
     leg too), 302 to the corp/Vercel AS.
   - `/oauth/oidc/callback?code=&state=` → exchange the code at the upstream token endpoint, verify
     the `id_token` against the upstream JWKS (or call `userinfo`), map to
     `MintedClaims { sub, email, email_verified }`, then reuse `bindCodeToFlow` → back to the client
     with Temper's `?code`. From here the existing `/oauth/token` mints the Temper JWT unchanged.
3. **`handleAuthorize` branch:** choose `/oauth/saml/login` vs `/oauth/oidc/login` based on the
   instance's configured upstream (or support both and pick per-connector).
4. **Membership reconcile:** reuse `reconcileMemberships` with `provider: "oidc:<idp_key>"`, sourcing
   groups from the upstream `groups` claim (same null-vs-empty signal-missing guard as SAML).
5. **Discovery & Rust config:** unchanged. Instance stays in `TemperAs` mode (`AS_ISSUER` set); the
   Rust `AUTH_ISSUER`/`JWKS_URL`/`AUTH_AUDIENCE` still point at the Temper AS. `oauth/metadata.ts`
   already advertises the Temper AS endpoints in AS mode — no client-facing change.
6. **Passport on the UI (optional, parallel):** enable Passport on the `temper-ui` Vercel project,
   pointed at the same corp IdP; read `x-vercel-oidc-passport-token` (verify vs Vercel JWKS, trust
   `external_sub`) if the UI's server needs the identity. The API/MCP project stays ungated.

Effort is concentrated in `temper-cloud` (TypeScript) + one migration; the Rust workspace is
essentially untouched. That is the payoff of Temper already being its own AS.

---

## Part 6 — Decisions

**Resolved (2026-07-20):**

1. **Approach = B1** — federate the *corporate IdP* as an upstream `oidc` connector behind Temper's
   AS. Not B2 (Vercel accounts as identity).
2. **temperkb.io is unchanged** — stays `ExternalIdp`/Auth0 (community/simple OAuth shape).
3. **SAML-direct stays viable** — B1 is additive; orgs that don't want Vercel as a front door keep
   `saml` (or `oidc` → their own IdP). Nothing is retired.
4. **Deployment posture becomes first-class and typed** (Part 4b) — two axes, a typed
   `kb_auth_connector` (not a KV `kb_system_settings`), crypto identity stays in env.
5. **"Vercel OIDC" = the `oidc` connector pointed at Vercel**, not a distinct type. Taxonomy stays at
   two connector types (`saml`, `oidc`). (The workload-identity `getVercelOidcToken` is machine→cloud,
   not user identity, and is out of scope.)
6. **`deployment_mode` is an env var** — resolved next to `parse_auth_config`, set-once boot posture,
   never twiddled at runtime; fail-closed if it disagrees with the env posture / active connector.
7. **Upstream OIDC `client_secret` lives in env** (one deployment secret, like `AS_*`), referenced by
   name from `kb_oidc_idp.client_secret_ref`; distinct prefix from `AS_*` (e.g.
   `OIDC_UPSTREAM_CLIENT_SECRET`) to keep "our AS" and "the upstream IdP" separate. No vault, no DB
   secret column.

**Open:**

1. **Passport on the web UI now or later?** Additive on the separate `temper-ui` project; not on the
   critical path for CLI/API/MCP.
2. **Agent/MCP identity roadmap:** the Vercel agent-credential connection layer is beta; Temper's own
   `client_credentials` machine principals remain the M2M path and are unaffected.

---

## Sources

External (Vercel):

- [Restrict access to deployments with Passport](https://vercel.com/docs/passport)
- [The Complete Guide to Vercel Passport](https://vercel.com/kb/guide/vercel-passport)
- [Vercel Passport is now in Public Beta](https://vercel.com/changelog/vercel-passport-is-now-in-public-beta)
- [Sign in with Vercel](https://vercel.com/docs/sign-in-with-vercel) · [Authorization Server API](https://vercel.com/docs/sign-in-with-vercel/authorization-server-api) · [Tokens](https://vercel.com/docs/sign-in-with-vercel/tokens) · [Agent quickstart](https://vercel.com/docs/sign-in-with-vercel/agent-quickstart)
- [Deploy MCP servers to Vercel (withMcpAuth / protected-resource metadata)](https://vercel.com/docs/mcp/deploy-mcp-servers-to-vercel)
- [Automated agent access / deployment-protection bypass](https://vercel.com/docs/deployment-protection/automated-agent-access) · [Trusted Sources](https://vercel.com/docs/deployment-protection/methods-to-bypass-deployment-protection/trusted-sources)
- [Vercel: internal apps & AI agents behind the corporate IdP (coverage)](https://idtechwire.com/vercel-puts-internal-apps-and-ai-agents-behind-the-corporate-identity-provider/)

Internal (Temper, current state — file:line):

- Mode switch & config parser: `crates/temper-services/src/auth_config.rs`
- Shared JWKS validator: `crates/temper-services/src/state.rs`
- Shared auth seam / machine principals: `crates/temper-services/src/auth/{mod,normalize}.rs`
- Owned OAuth AS + SAML round-trip: `packages/temper-cloud/src/oauth/endpoints.ts`
- Token minting (claim shape): `packages/temper-cloud/src/oauth/mint.ts`
- SAML SP (the template to mirror): `packages/temper-cloud/src/saml/sp.ts`, `packages/temper-cloud/src/saml/config.ts`
- RFC 8414 AS metadata (mode branch): `packages/temper-cloud/src/oauth/metadata.ts`
- MCP discovery + DCR proxy: `crates/temper-mcp/src/discovery.rs`, `crates/temper-mcp/src/middleware.rs`
- CLI login (auth-code + PKCE): `crates/temper-client/src/login.rs`
- Deployment topology: `vercel.json`
- Prior federation design: `docs/superpowers/specs/2026-07-01-saml-sp-temper-authorization-server-design.md`
