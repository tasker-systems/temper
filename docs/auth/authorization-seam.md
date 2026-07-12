# The authorization seam (`temper-services::auth`)

The seam is where the **order of authorization gates** lives — exactly once, shared by
both surfaces. `temper-api` and `temper-mcp` both call it and map its errors to their own
transport; neither re-implements the sequence.

Source: `crates/temper-services/src/auth/mod.rs` (the chain),
`auth/normalize.rs` (classification), `auth/email.rs` (the human email ladder).

## The seam owns principal construction

A surface hands in a **verified token** and gets back an `AuthenticatedProfile`. It never
builds an `AuthClaims` — and nothing in the seam's public API accepts one.

This is the level below the one classification closed. Making `classify` total (PR #384)
meant no surface could say *"unrecognized ⇒ human"* — but each surface still hand-built its
own **human** `AuthClaims`, and the two disagreed: temper-api ran a three-rung email ladder,
temper-mcp set `email: ""` and auto-provisioned. Any surface that can construct an
`AuthClaims` can construct a `PrincipalKind::Human`, which is precisely the asymmetry that
made #384's bug asymmetric in the first place. One constructor, one ladder, one answer per
token.

**Forgery is inert, not forbidden.** `AuthClaims` is still a public type a surface *can*
construct. But `authenticate`, `classify`, `Principal` and `resolve_from_claims` are all
crate-private, so a hand-built `AuthClaims` has nowhere to go — there is no public function
that takes one. The enforcement is the absence of a door, not a runtime check.

## Three public entry points, a typestate chain

```rust
// THE TOKEN PATH — Level 1. Verified RawJwtClaims + the raw bearer → resolved, ACTIVE
// profile. Runs on EVERY authed request on BOTH surfaces. The seam classifies, runs the
// human email ladder, constructs the AuthClaims, resolves, and gates.
pub async fn authenticate_token(
    state: &AppState,
    raw: &RawJwtClaims,
    token: &str,
) -> Result<AuthenticatedProfile, AuthzError>;

// THE FEDERATED PATH — an identity a trusted peer already authenticated out-of-band
// (the SAML reconcile channel: HMAC over the body, no JWT, nothing to classify).
pub async fn resolve_federated_human(
    pool: &PgPool,
    provider: &str,          // server config, NEVER a payload field
    external_user_id: &str,
    email: &str,
    email_verified: Option<bool>,
) -> ApiResult<Profile>;

// Level 2 — system authorization. Consumes proof of Level 1, adds the access gate.
// Runs on the GATED tier of BOTH surfaces.
pub async fn require_system_access(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError>;
```

Level 1's gate function itself — `authenticate(pool, &claims)` — is **`pub(crate)`**. It is
reachable only through `authenticate_token`, which is what makes "the seam owns the
principal" true by construction rather than by convention.

- `authenticate_token` classifies (`classify`), runs the email ladder on the human arm
  (`email::resolve_email_from_claims`), builds the `AuthClaims`, and calls `authenticate`.
  The **machine arm deliberately skips the ladder** — an M2M principal has no email and no
  `/userinfo` to ask, so a ladder there would be an authentication failure dressed as a
  lookup. That ordering is load-bearing; it is pinned by
  `machine_token_authenticates_without_running_the_email_ladder`.
- `authenticate` runs `resolve_from_claims` (which JIT-provisions a *human* profile on first
  sight, and is **lookup-or-reject** for a machine — see the
  [machine-token contract](./machine-token-contract.md)) then the `is_active` gate. It
  yields `AuthenticatedProfile { profile, claims }`.
- `require_system_access` runs `access_service::has_system_access` (approved member of the
  gating team). It yields `SystemAuthorized(AuthenticatedProfile)`.

**Typestate, not a marker bool.** `require_system_access` only accepts an
`AuthenticatedProfile`, and that type is *only* produced by `authenticate`. So the type
system proves Level 1 ran before Level 2 — you cannot call the system-access gate on an
unauthenticated principal. This is parse-don't-validate: a call site that needs system
access asks for `SystemAuthorized`, and possessing that value *is* the proof the gate ran.
It is deliberately lightweight (`SystemAuthorized` is a thin newtype wrapping the already-
existing `AuthenticatedProfile`), not a heavyweight typestate framework.

## Why two levels, not one monolith

`temper-api` splits into two router tiers (see `routes.rs`):

- **auth-only tier** — view own profile, request access, `team join`. Runs Level 1
  (`require_auth`) but deliberately **skips** Level 2. This is how a not-yet-approved user
  requests access in the first place.
- **gated tier** — everything else. Adds the `require_system_access` layer.

`temper-mcp` has **no** auth-only tier: every tool requires Level 2.

A single `authorize_request()` that always ran all gates would **break** the request-access
flow. The honest model is the two-level chain, each level single-sourced. A future gate
belongs to exactly one level, so "add a gate = edit one function" holds *per level* — the
real, defensible version of the acceptance criterion.

## `AuthzError` — one enum, mapped per transport

The seam speaks the vocabulary of *why* a request was refused. It never chooses the
words-on-the-wire; each surface owns that mapping and only that.

```rust
pub enum AuthzError {
    Refused(&'static str),            // classification refused the token (machine-shaped,
                                      //   but not coherently so). Carries the reason.
    EmailResolution(ApiError),        // the human email ladder fell off its bottom rung
    ProfileResolution(ApiError),      // resolve_from_claims failed (DB, missing link,
                                      //   unregistered/revoked machine client, …)
    AccessCheck(ApiError),            // has_system_access check itself failed (DB error)
    Deactivated { profile_id },       // is_active == false
    SystemAccessDenied { profile_id },// not an approved member of the gating team
}
```

`AccessCheck` is kept distinct from `SystemAccessDenied` so a surface can preserve the
pre-seam *"failed to check system access"* diagnostic instead of collapsing an infra
failure into a clean access-denied message. For the same reason `EmailResolution` is kept
distinct from `ProfileResolution`: nothing was resolved — we could not even *name* the
human, so no write was attempted.

### Transport mapping

| `AuthzError` | temper-api | temper-mcp |
|--------------|-----------|-----------|
| `Refused` | `401 Unauthorized` — `"Invalid or expired token"` (the reason is logged with the `sub`, never put on the wire) | `INVALID_REQUEST`, terminal ("do not retry") |
| `EmailResolution(e)` | the inner `ApiError` (a `401`) | `INVALID_REQUEST`, terminal — a retry resolves nothing; the fix is a token carrying an email claim |
| `Deactivated` | `401 Unauthorized` — `"account is deactivated"` | `INVALID_REQUEST`, terminal |
| `SystemAccessDenied` | `ApiError::SystemAccessRequired { details }` — carries `SystemAccessDetails` (email, display_name, access_mode, join-request status, request URL, CLI command) | `INVALID_REQUEST` with the request-access guidance text |
| `ProfileResolution(ApiError::Unauthorized)` | the inner `ApiError` (a `401`) | `INVALID_REQUEST`, **terminal** — this is usually the machine-registration gate denying an unregistered or revoked `client_id`, a permanent denial a Sidekiq-style client must not retry |
| `ProfileResolution(e)` / `AccessCheck(e)` (any other) | the inner `ApiError` | `internal_error` (retryable — a genuine infra fault) |

temper-api mappers: `middleware/auth.rs` (Level 1) and `middleware/system_access.rs`
(Level 2). temper-mcp mapper: `service.rs::map_authz_error`.

Both Level 2 mappers still have to *spell* the Level 1 variants (`Refused`,
`EmailResolution`) because the enum is shared and `match` is exhaustive; they map them to an
internal error, since `require_system_access` neither classifies a token nor resolves an
email. That arm is defensively unreachable, not live.

> The `SystemAccessDetails` payload reflects the caller's **own** profile data
> (email/display_name) back to them — safe, because OAuth already proved they own that
> identity. See the SECURITY NOTE in `middleware/system_access.rs`.

## How each surface wires in

**temper-api.** `require_auth` verifies the JWT, decodes it into `RawJwtClaims`, then calls
`authenticate_token(&state, &raw, &token)` — classification, the human email ladder, claim
construction and the deactivation gate all happen inside the seam — and injects the
resulting `AuthenticatedProfile` into request extensions. The gated router adds a
`require_system_access` **layer** that reads that extension and calls the seam's Level 2 for
its gate effect.

> **Note (typestate on temper-api is partial).** Because temper-api enforces Level 2 as a
> router *layer* rather than inside a handler, the `SystemAuthorized` token cannot easily
> reach handlers as a value — the layer runs the gate for effect and discards the token.
> The typestate benefit is fully realized on temper-mcp's imperative call site and only
> partially on temper-api. Handlers continue to read `AuthenticatedProfile` from
> extensions via the `AuthUser` extractor.

**temper-mcp.** `require_mcp_auth` verifies the JWT and injects **two** things into the HTTP
extensions: the decoded `RawJwtClaims` and the raw `BearerToken` (the ladder's `/userinfo`
rung needs the token itself, not its claims). `ensure_profile_from_parts` — called at the top
of every tool — pulls both back out and calls `authenticate_token` then
`require_system_access` back to back, caching the resolved profile for the tool body. Both
refusals route through `map_authz_error`.

**temper-api's internal SAML reconcile handler** is the third caller, on the federated path:
`handlers/internal_saml.rs` calls `resolve_federated_human`. See
[reconcile-channel.md](./reconcile-channel.md).

## Two MCP behaviors this closed

Both were live gaps on temper-mcp only, and both are consequences of the surface no longer
constructing its own principal.

- **An unnamable human is now refused, not auto-provisioned.** temper-mcp previously set
  `email: String::new()` and skipped the ladder entirely, so a human token with no `email`
  claim and no cached auth link created a junk profile with an empty email. It now runs the
  one ladder like temper-api and refuses with `EmailResolution` — *before any write*. A human
  we cannot name is a human we will not provision. (A returning MCP human with no `email`
  claim still works: rung 2, the cached `kb_profile_auth_links` email from an earlier
  sign-in, answers.) Pinned by
  `emailless_unlinked_human_is_refused_without_provisioning` and
  `human_without_email_claim_resolves_from_cached_link`.
- **`initialize` no longer skips the deactivation gate.** MCP's `initialize` used to call
  `resolve_from_claims` directly as a best-effort cache seed, which bypassed Level 1's
  `is_active` check: a deactivated account was refused on every *tool call* but could still
  **open an MCP session**. It now goes through `authenticate_token`, and a refusal there
  propagates rather than being warned past. (`initialize` runs Level 1 only; Level 2 still
  runs per tool call in `ensure_profile_from_parts`.)

## The parity test — at the production caller's level

`tests/e2e/tests/auth_seam_parity_e2e.rs` proves a deactivated profile and a no-access
profile are refused **identically** on both surfaces:

- The **API** surface is driven over HTTP through the real middleware stack.
- The **MCP** surface is driven by constructing a `TemperMcpService` over the same test
  pool and calling the production gate `ensure_profile_from_parts` with hand-built request
  `Parts` carrying `RawJwtClaims` + `BearerToken` — exactly what `require_mcp_auth` injects.

This is the test the per-surface `is_active` gap would have failed. A direct-call unit test
over `authenticate_token` / `authenticate` / `require_system_access` (which also exists, in
the seam module — those tests are in-crate, so they can reach the `pub(crate)` gate) proves
the *decisions*, but it passes even if a surface forgot to wire the seam in — the e2e is
what proves the wiring. Keep both.

`tests/e2e/tests/auth_seam_m2m_e2e.rs` is the same discipline for machine tokens: it drives
the real MCP gate and asserts an unregistered client is rejected and creates nothing, a
registered Auth0 client is admitted, and a **temper-issued** client resolves on MCP too.

Run them: `cargo make test-e2e` (both need only `test-db`).
