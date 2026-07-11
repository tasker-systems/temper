# The authorization seam (`temper-services::auth`)

The seam is where the **order of authorization gates** lives — exactly once, shared by
both surfaces. `temper-api` and `temper-mcp` both call it and map its errors to their own
transport; neither re-implements the sequence.

Source: `crates/temper-services/src/auth/mod.rs`.

## Two functions, a typestate chain

```rust
// Level 1 — authentication. Verified+normalized claims → resolved, ACTIVE profile.
// Runs on EVERY authed request on BOTH surfaces.
pub async fn authenticate(
    pool: &PgPool,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError>;

// Level 2 — system authorization. Consumes proof of Level 1, adds the access gate.
// Runs on the GATED tier of BOTH surfaces.
pub async fn require_system_access(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError>;
```

- `authenticate` runs `resolve_from_claims` (JIT-provisions the profile on first sight)
  then the `is_active` gate. It yields `AuthenticatedProfile { profile, claims }`.
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
    ProfileResolution(ApiError),      // resolve_from_claims failed (DB, missing link, …)
    AccessCheck(ApiError),            // has_system_access check itself failed (DB error)
    Deactivated { profile_id },       // is_active == false
    SystemAccessDenied { profile_id },// not an approved member of the gating team
}
```

`AccessCheck` is kept distinct from `SystemAccessDenied` so a surface can preserve the
pre-seam *"failed to check system access"* diagnostic instead of collapsing an infra
failure into a clean access-denied message.

### Transport mapping

| `AuthzError` | temper-api | temper-mcp |
|--------------|-----------|-----------|
| `Deactivated` | `401 Unauthorized` — `"account is deactivated"` | `INVALID_REQUEST`, terminal ("do not retry") |
| `SystemAccessDenied` | `ApiError::SystemAccessRequired { details }` — carries `SystemAccessDetails` (email, display_name, access_mode, join-request status, request URL, CLI command) | `INVALID_REQUEST` with the request-access guidance text |
| `ProfileResolution(e)` / `AccessCheck(e)` | the inner `ApiError` | `internal_error` |

temper-api mappers: `middleware/auth.rs` (Level 1) and `middleware/system_access.rs`
(Level 2). temper-mcp mapper: `service.rs::map_authz_error`.

> The `SystemAccessDetails` payload reflects the caller's **own** profile data
> (email/display_name) back to them — safe, because OAuth already proved they own that
> identity. See the SECURITY NOTE in `middleware/system_access.rs`.

## How each surface wires in

**temper-api.** `require_auth` verifies the JWT, runs the email-resolution ladder, builds
`AuthClaims`, then calls `authenticate` and injects the resulting `AuthenticatedProfile`
into request extensions. The gated router adds a `require_system_access` **layer** that
reads that extension and calls the seam's Level 2 for its gate effect.

> **Note (typestate on temper-api is partial).** Because temper-api enforces Level 2 as a
> router *layer* rather than inside a handler, the `SystemAuthorized` token cannot easily
> reach handlers as a value — the layer runs the gate for effect and discards the token.
> The typestate benefit is fully realized on temper-mcp's imperative call site and only
> partially on temper-api. Handlers continue to read `AuthenticatedProfile` from
> extensions via the `AuthUser` extractor.

**temper-mcp.** `ensure_profile_from_parts` (called at the top of every tool) builds
`AuthClaims` from the injected `RawJwtClaims` (via `classify`, whose closed-sum return forces
this surface to handle the machine branch AND the refusal branch — see
[machine-token-contract.md](./machine-token-contract.md)), calls
`authenticate` then `require_system_access` back to back, and caches the resolved profile for
the tool body. Both refusals route through `map_authz_error`.

## The parity test — at the production caller's level

`tests/e2e/tests/auth_seam_parity_e2e.rs` proves a deactivated profile and a no-access
profile are refused **identically** on both surfaces:

- The **API** surface is driven over HTTP through the real middleware stack.
- The **MCP** surface is driven by constructing a `TemperMcpService` over the same test
  pool and calling the production gate `ensure_profile_from_parts` with hand-built request
  `Parts` carrying `RawJwtClaims`.

This is the test the per-surface `is_active` gap would have failed. A direct-call unit test
over `authenticate` / `require_system_access` (which also exists, in the seam module) proves
the *decisions*, but it passes even if a surface forgot to wire the seam in — the e2e is
what proves the wiring. Keep both.

Run it: `cargo make test-e2e` (the parity test needs only `test-db`).
