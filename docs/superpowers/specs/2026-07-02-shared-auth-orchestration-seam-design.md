# Shared auth-orchestration seam (temper-services `auth` module)

**Status:** Draft for review · **Date:** 2026-07-02 · **Task:** `019f22f9-716c-7123-9952-35528fcd1a39`
**Mode/effort:** plan / medium · **Branch:** `jct/auth-seam-spec`

> **Author's note (decisions made while Cole was away):** the scope question below was
> answered as **"one spec, sequenced build tasks"** on my best judgment. The open
> sub-decisions (does the seam own JWT verification; typestate shape; where the
> cross-surface test lives) are resolved inline with rationale and flagged **[DECISION]**
> so they are easy to review or override. Nothing here is built yet — this is the spec
> for a follow-on `writing-plans` pass.

## Problem

Authorization is enforced **twice, in parallel**, once per surface. The shared *logic*
already lives in temper-services (`profile_service`, `access_service`), but the
**orchestration** — the ordered sequence of gates — is hand-assembled separately in each
surface, so a gate added to one surface silently misses the other.

Concretely, as of today:

| Gate | temper-api | temper-mcp |
|------|-----------|-----------|
| JWT verify (JWKS decode) | `middleware/auth.rs::require_auth` (audience = `auth_audience`) | `middleware.rs::require_mcp_auth` (audience = `mcp_audience`) |
| Claim → `AuthClaims` normalization | `require_auth` (resolves email via claim/cache/userinfo) | `service.rs::resolve_profile` (email left empty, resolved downstream) |
| `resolve_from_claims` | `require_auth` | `ensure_profile_from_parts` |
| `is_active` gate | `require_auth` | `ensure_profile_from_parts` (added as a **hotfix** after the 2026-07-02 review found it missing) |
| `system_access` gate | **separate** `middleware/system_access.rs`, applied only to the *gated* router | `ensure_profile_from_parts`, applied to *every* tool |

**The `is_active` miss was real, not hypothetical.** SAML Phase 2 added the deactivation
gate to temper-api only; the consolidated review (2026-07-02, IMPORTANT-1) caught that
temper-mcp had none — a deactivated account's valid token kept full MCP tool access. A
3-line MCP gate was hotfixed, but the *structural* gap remains: the next gate can miss a
surface just as easily.

### Structural finding: the sequence is NOT identical across surfaces

This shapes the whole design. temper-api splits into **two router tiers**:

- **auth-only router** (view own profile, request access, `team join`) — runs
  `require_auth` (verify → resolve → `is_active`) but deliberately **skips** `system_access`.
  This is how a not-yet-approved user requests access in the first place.
- **gated router** (everything else) — adds the `require_system_access` layer.

temper-mcp has **no** auth-only tier — every tool requires `system_access`.

So a single monolithic `authorize_request()` that always runs all three gates would
**break** temper-api's request-access flow. The honest model is a **two-level chain**,
each level single-sourced:

1. **Authenticated** = valid token + resolved profile + `is_active`. (Both surfaces, all authed routes.)
2. **System-authorized** = Authenticated + `has_system_access`. (Both surfaces' gated tiers.)

A future gate belongs to exactly one level, so "add a gate = edit one function" holds
*per level* — which is the real, defensible version of the acceptance criterion.

## Proposal: an `auth` module in temper-services

**Home — DECIDED (Cole, 2026-07-02):** an `auth` module inside `temper-services`, **not** a
new `temper-auth` crate. temper-services is already the shared business-logic + auth-infra
layer for both surfaces; both already depend on it. Revisit a dedicated crate only if a
third consumer beyond api/mcp ever appears.

### Shape: two functions forming a typestate chain

```rust
// crates/temper-services/src/auth/mod.rs  (new module)

/// Level 1 — authentication. Verified+normalized claims → resolved, active profile.
/// Runs on EVERY authed request on BOTH surfaces.
///   resolve_from_claims  →  is_active gate
pub async fn authenticate(
    pool: &PgPool,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError>;

/// Level 2 — system authorization. Consumes proof of Level 1, adds the access gate.
/// Runs on the gated tier of BOTH surfaces.
///   has_system_access
pub async fn require_system_access(
    pool: &PgPool,
    profile: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError>;
```

**[DECISION] Typestate over marker booleans.** `require_system_access` takes
`&AuthenticatedProfile` (only obtainable from `authenticate`) and returns a
`SystemAuthorized` token. This is parse-don't-validate: a handler that needs system
access asks for `SystemAuthorized`, and the *type* proves the gate ran — you cannot call
Level 2 without having passed Level 1. Keep it lightweight: `AuthenticatedProfile` already
exists in temper-core; `SystemAuthorized` is a thin newtype wrapping it. Not a heavyweight
typestate framework.

### `AuthzError` — one error enum, mapped per transport

```rust
pub enum AuthzError {
    ProfileResolution(ApiError),   // resolve_from_claims failed (DB, etc.)
    Deactivated { profile_id },    // is_active == false
    SystemAccessDenied { details: SystemAccessDetails },  // has_system_access == false
}
```

Each surface owns the mapping to its transport, and **only** the mapping:

- temper-api: `Deactivated → 401`, `SystemAccessDenied → ApiError::SystemAccessRequired`
  (preserving the existing `SystemAccessDetails` body: email/display_name/access_mode/…).
- temper-mcp: `Deactivated → rmcp INVALID_REQUEST` (terminal, "do not retry"),
  `SystemAccessDenied → rmcp INVALID_REQUEST` with the request-access guidance text.

The gate *sequence* and *decisions* live once in `auth`; only the words-on-the-wire differ.

### What each surface keeps vs. delegates

| Concern | After the seam |
|---------|----------------|
| JWT signature verify (JWKS decode) | **Stays per-surface for the first cut** — audience differs legitimately (`auth_audience` vs `mcp_audience`). See M2M stage for the follow-on. |
| Claim → `AuthClaims` normalization | temper-api keeps its email-resolution ladder (claim → cache → userinfo); temper-mcp keeps its empty-email path. Normalization is *input* to the seam. **M2M stage** introduces a shared normalizer because machine tokens change the claim shape. |
| `authenticate` (resolve + is_active) | **Delegated to the seam** — both surfaces call it. |
| `require_system_access` | **Delegated to the seam** — temper-api's `middleware/system_access.rs` and temper-mcp's `ensure_profile_from_parts` both call it. |

**[DECISION] JWT-verify unification is a follow-on, not the first cut.** The task's own
framing ("unifying just the profile+gates step is the smaller, higher-value first cut")
wins for stages 1–2. The M2M stage (4) then pulls verification into the seam *because it
must* — machine tokens carry a different claim shape (`azp`/`client_id`, audience, no human
`sub`/email), and normalizing that is a seam concern. So verify-in-the-seam arrives when it
earns its keep, not speculatively.

## Sequenced deliverables

One coherent vision, shipped as separate PR-sized build tasks. Recommended order:

### Stage 1 — the seam + cross-surface parity test  *(core; unblocks nothing external but is the spine)*

- New `temper-services/src/auth/` module: `authenticate`, `require_system_access`,
  `AuthzError`, `SystemAuthorized`.
- Rewire temper-api `require_auth` → call `authenticate`; `system_access.rs` →
  call `require_system_access`. No behavior change on the happy path.
- Rewire temper-mcp `ensure_profile_from_parts` → call `authenticate` then
  `require_system_access`. The inline is_active + has_system_access blocks collapse into
  two seam calls.
- **[DECISION] Cross-surface parity test lives in two layers:**
  1. A temper-services unit/integration test over `authenticate` + `require_system_access`
     proving the *decisions* (deactivated → refused; no-access → refused; active+member →
     allowed). This is the single source of gate truth.
  2. An **e2e test per surface** (tests/e2e) that drives the *production caller* — a real
     request through temper-api's middleware and a real MCP tool call — asserting a
     deactivated profile and a no-access profile are refused identically on both. This is
     the test the per-surface `is_active` gap would have failed (per the "e2e at the
     production caller's level" discipline). A direct-call-only test passes even if a
     surface forgot to wire the seam in — the e2e is what proves the wiring.
- Acceptance: "add a hypothetical new gate = edit one function" is demonstrably true for
  each level; both surfaces refuse deactivated + no-access accounts.

### Stage 2 — `docs/auth/` area  *(documentation; can land with or right after Stage 1)*

Stand up `docs/auth/` as the canonical home for Temper's security + auth flows, so future
auth changes have somewhere to live and the "did I touch both surfaces?" checklist is
written down. Cover:

- The two enforcement surfaces (temper-api middleware + temper-mcp) and the shared
  `temper-services` auth seam.
- The two-level chain (authenticated / system-authorized) and the router-tier split.
- JWT verification (Auth0/OIDC + the SAML AS EdDSA path, `JwksKeyStore`).
- The internal reconcile channel + its secret/HMAC trust model, **including the
  "why not an origin allow-list on Vercel" reasoning** (server-side `fetch` sends no
  `Origin`; egress IPs aren't pinnable; the secret *is* the sibling-trust signal).
- Profile deactivation (`is_active`) as the authn lever; `system_access` gating.

Move/replace the scattered auth notes currently only in `self-hosting-saml.md` and crate
CLAUDE.md files into this area (leave forward-pointers where notes are removed).

### Stage 3 — internal reconcile channel HMAC hardening  *(independent; same auth area)*

Today `POST /internal/saml/reconcile` (AS → temper-api) is gated by a **static shared
secret** in `X-Temper-Internal-Secret` (constant-time compared, fail-closed when unset).
That is the *correct origin control for Vercel serverless* — an IP/origin allow-list would
be security theater there. Two real hardening levers:

- **HMAC + timestamp request signing** (the upgrade): the AS signs
  `HMAC(secret, canonical_body ‖ timestamp)`; the API verifies the MAC and rejects stale
  timestamps (~30s window). Wins over the raw header: the secret never travels the wire,
  and it becomes **replay-proof**. Same trust model, meaningfully hardened. This is the
  honest version of "same-origin only" for this topology.
- Strong/rotated `INTERNAL_RECONCILE_SECRET` (≥32 random bytes; **document rotation**) +
  edge rate-limiting (Vercel Firewall/WAF) on the path.
- **Record the bounded blast radius** in the docs: even if reached, the endpoint can only
  apply operator-pre-configured `kb_saml_group_mappings` (no arbitrary grants) and never
  touches `native` memberships.

**Out of scope (explicit):** a true network boundary (making the API non-publicly-routable)
— it conflicts with the API also serving public OAuth/SAML endpoints and is Enterprise-tier
Vercel; not worth it vs. the HMAC upgrade.

**Implementation note:** the AS side is TypeScript (temper-cloud); the verify side is the
Rust `internal_auth.rs` middleware. The canonical-body + timestamp contract is a shared
wire concern — define it once (a small typed struct in temper-core with ts-rs, or at
minimum a documented canonicalization) so the two sides can't drift on byte order.

### Stage 4 — M2M `client_credentials` for agent principals  *(highest external urgency: unblocks T6 steward)*

**Why this is the T6 deploy blocker:** the deployed steward authenticates via a Vercel
Connect connector with `principalType: "app"` (a machine acting as itself, no user). But
the OAuth AS advertises only `authorization_code` + `refresh_token` — **no
`client_credentials`** — so an app principal can never mint a token → the MCP connection
never establishes → zero MCP calls → no profile created. Verified 2026-07-02.

Fold into the seam because it is exactly a resolve+authorize concern:

- **Advertise `client_credentials`** in the OAuth authorization-server metadata. **Note the
  location:** RFC 8414 `/.well-known/oauth-authorization-server` (where
  `grant_types_supported` lives) is served by the **temper-cloud AS layer (TypeScript)**,
  not temper-mcp's Rust `discovery.rs` (which only serves protected-resource metadata +
  the DCR proxy). So this advertisement is a temper-cloud change. Provision an **Auth0 M2M
  application** authorized for the temper API audience.
- **`authenticate` must handle a machine token.** No human `sub`/email — the `sub` is the
  client id. The **shared claim normalizer** (introduced here) maps the M2M claim shape
  (`azp`/`client_id`, audience, no email) into `AuthClaims`, and `resolve_from_claims`
  maps the client → a **dedicated agent profile** (provisioned on first sight, keyed by the
  client id via a `kb_profile_auth_links` row with a machine provider). The agent is its own
  accountable principal — fits the invocation-envelope model — never a proxied human.
- **This is where JWT-verify enters the seam.** Because M2M tokens need claim-shape
  normalization that both surfaces must share, the seam grows a `verify_and_normalize`
  entry that owns JWKS decode + claim-shape branching (human vs machine), audience passed
  in by the caller. The per-surface verify blocks from Stage 1 collapse here.
- The agent's provisioned profile then takes **ordinary grants** like any principal: team
  membership for source read + `cogmap grant --write` for authoring. No special-casing in
  the gates — a machine profile passes `is_active` and `system_access` on the same rails as
  a human.

**Bridge, if the steward must go live before Stage 4 lands:** `authorization_code +
refresh_token` as a dedicated steward login works with temper-mcp as-is (one-time browser
consent; see `docs/guides/vercel-eve.md`). Own principal, needs a real login identity +
interactive consent. The M2M grant is the correct destination; the bridge is the escape
hatch. (**Avoid** the `user`-subject-as-Cole path — it proxies-as-Cole and conflates
authorship, which Cole explicitly rejected.)

## Components affected

- **New:** `crates/temper-services/src/auth/` (module: `authenticate`,
  `require_system_access`, `AuthzError`, `SystemAuthorized`; + Stage 4:
  `verify_and_normalize`, machine-claim normalizer).
- **temper-api:** `middleware/auth.rs` (delegate resolve+is_active),
  `middleware/system_access.rs` (delegate the access gate). Transport mapping only.
- **temper-mcp:** `service.rs::ensure_profile_from_parts` (delegate both levels),
  `middleware.rs` (Stage 4: hand verify to the seam).
- **temper-services:** `profile_service::resolve_from_claims` (Stage 4: machine-principal
  provisioning branch), `access_service` (unchanged; already the gate logic).
- **temper-cloud (TS):** AS metadata `grant_types_supported += client_credentials`
  (Stage 4); HMAC signing on the reconcile call (Stage 3).
- **temper-core:** wire contracts — machine `AuthClaims` variant / reconcile
  canonical-body struct (ts-rs shared).
- **docs/auth/** (Stage 2, new area).
- **tests/e2e:** cross-surface parity suite (Stage 1).

## Key decisions & trade-offs accepted

- **Two-level chain, not a monolith** — forced by the auth-only vs gated router split.
  Trade-off: "one function" becomes "one function per level"; accepted because the levels
  are genuinely distinct authorization states and the alternative breaks request-access.
- **Typestate (`SystemAuthorized`) over a bool flag** — the compiler enforces that Level 2
  can't run without Level 1. Small ceremony cost; buys a class of wiring bugs eliminated.
- **JWT verify unified only at Stage 4** — avoids speculative abstraction; verification
  centralizes exactly when M2M claim-shape divergence makes it pay for itself.
- **Machine principals ride the ordinary gate rails** — no auth-path special-casing; an
  agent profile is `is_active` + `system_access` + grants like anyone. Keeps the
  invocation-accountability model clean.
- **Sequenced, not one PR** — four PR-sized stages; Stage 1 is the spine, Stage 4 has the
  external urgency (T6). Avoids a mega-PR that's hard to review and revert.

## Open questions / risks

1. **Scope confirmation.** I chose "one spec, sequenced build tasks." If Cole wants
   M2M-first (T6 urgency) driving Stage 1's shape, Stage 1 and Stage 4 partially merge —
   verify-in-seam would land immediately rather than as a follow-on. Flagged for review.
2. **Machine-provider modeling.** Does the agent profile use a distinct `auth_provider`
   value (e.g. `"m2m"`/`"auth0-m2m"`) in `kb_profile_auth_links`, and does
   `resolve_from_claims` branch on provider or on claim shape? Leaning provider-tagged so
   the machine path is explicit and auditable. Resolve during Stage 4 design.
3. **`SystemAuthorized` plumbing on temper-api.** temper-api applies system_access as a
   router *layer*, not inside a handler, so the `SystemAuthorized` token can't easily reach
   handlers as a value. Likely temper-api keeps injecting `AuthenticatedProfile` into
   extensions and the layer just *runs* `require_system_access` for its gate effect. The
   typestate benefit is fully realized on temper-mcp (imperative call site) and partially on
   temper-api (layer). Acceptable; note it.
4. **HMAC canonicalization contract.** Byte-order/whitespace of `canonical_body` must be
   pinned across a TS signer and a Rust verifier. Risk of drift; mitigated by a shared typed
   contract + a round-trip test fixture.
5. **Auth0 M2M provisioning** is operator/console work outside the repo — the spec covers
   what the code must accept, but the Auth0 app + audience grant is a manual runbook step
   (document it in docs/auth/).

## Follow-up build tasks (to create after spec approval)

- **Stage 1** — build/medium: "Extract `temper-services::auth` two-level seam + cross-surface parity test"
- **Stage 2** — build/small: "Stand up `docs/auth/` canonical area; migrate scattered auth notes"
- **Stage 3** — build/medium: "HMAC + timestamp signing on the internal reconcile channel"
- **Stage 4** — build/medium: "M2M `client_credentials` agent principals (advertise grant, Auth0 M2M app, machine-claim normalization, agent-profile provisioning) — unblocks T6 steward"
