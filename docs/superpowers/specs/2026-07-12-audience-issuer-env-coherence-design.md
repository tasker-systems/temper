# Audience/issuer env coherence — parse once, fail closed, name the variable

**Date:** 2026-07-12
**Goal:** `019f4910` — temper-rb (auth posture work bound up with the M2M arc)
**Task:** `019f5623-0ed2` · **Mode/effort:** plan / medium · **Branch:** `jct/audience-issuer-env-coherence`
**Status:** designed 2026-07-12
**Follows:** PR #391 (form-encoded M2M mint + auth docs), #384/#388 (the auth seam)

## Problem

Four environment variables have to agree, and nothing makes them agree.

| Var | Read by | Role |
|---|---|---|
| `AUTH_AUDIENCE` | temper-api resource server (`temper-services/src/config.rs:29`) | the `aud` the API validates |
| `MCP_AUDIENCE` | temper-mcp (`temper-mcp/src/config.rs:51-53`) | the `aud` MCP validates — **falls back to `AUTH_AUDIENCE`** |
| `AS_AUDIENCE` | temper's own AS, at mint (`packages/temper-cloud/src/oauth/mint.ts:37,62`) | the `aud` every AS-minted token carries |
| `AS_ISSUER` | temper's own AS (`mint.ts`, `metadata.ts`) | **its presence is the AS-mode signal** |

Plus `AUTH_ISSUER` and `JWKS_URL`, which in AS mode must point at that same AS.

### The security hole

`config.rs:29` resolves `AUTH_AUDIENCE` through `.ok().filter(|s| !s.is_empty())`, so **unset OR
empty ⇒ `None`**. `None` reaches `JwksKeyStore::validation` (`state.rs:148-152`), which sets
`validate_aud = false` — **audience validation is skipped entirely**. Line 34 logs a `warn` and
continues. A warning in a serverless log is not a control.

temper-mcp has no such hole: it always passes `Some(mcp_audience)`. So this is the
surface-asymmetry class the #384/#388 arc closed everywhere else, **surviving in configuration
rather than in code**.

### The asymmetry that proves one parser is needed

The *same* misconfiguration produces *opposite* failures:

- `AUTH_AUDIENCE=""` → temper-api filters empty to `None` → **validation disabled** (falls open).
- `MCP_AUDIENCE=""` → temper-mcp does **not** filter empty → enforces `aud == ""` → **rejects every
  token** (falls shut).

One typo, one variable, two opposite behaviors. Neither is a decision anyone made. Two parsers for
one concept is how this happened.

### Why "they must agree" was impossible to hold in your head

The agreement rule is **mode-dependent**, and nothing says so:

- The AS mints **every** token — human and machine — with a single `AS_AUDIENCE` (`mint.ts:37,62`;
  the pending flow stores it at `endpoints.ts:118`). So in **AS mode all three audiences collapse to
  one value**.
- Under an external IdP, `AUTH_AUDIENCE` and `MCP_AUDIENCE` *could* be two distinct API identifiers.

**Verified against both live deployments (2026-07-12): they are the same value everywhere.** Prod
(temperkb.io) is external-IdP mode with **no `AS_*` vars at all**; enterprise likewise sets both
audiences to one value. So the two-distinct-audiences case is **not real**, and `MCP_AUDIENCE`'s
silent fallback is pure risk with no upside: it exists to paper over a divergence that never occurs.

## Design

### 1. The fall-open branch becomes unconstructible

`auth_audience: Option<String>` → `audience: String`. `None` is what `state.rs:151` keys off to set
`validate_aud = false`; delete the `None` and the fall-open branch has nothing to branch on. Not
forbidden — **unrepresentable**. Same enforcement shape #388 used on `AuthClaims`: remove the
consumer, not the caller.

```rust
pub struct AuthConfig {
    pub issuer: String,
    pub jwks_url: String,
    pub audience: String,   // never Option — a missing audience cannot reach the validator
    pub mode: AuthMode,
}

pub enum AuthMode {
    ExternalIdp,  // AS_ISSUER unset — an external IdP (Auth0) fronts this instance
    TemperAs,     // AS_ISSUER set — temper mints its own tokens
}
```

`ApiConfig` swaps `auth_issuer` + `auth_audience` for one `auth: AuthConfig`.

`mode` is retained though nothing reads it post-parse: it lets boot emit `temper-api starting in
AS mode` / `in external-IdP mode`. An operator who cannot tell which mode their instance is in is
precisely the operator who mis-sets these variables. That is the goal of this task, not a nicety.

### 2. The rules

Enforced in `ApiConfig::from_env()` — the choke point **both** surfaces already call
(`api/axum.rs:25`, `api/mcp.rs:24`). Same seam precedent as the auth gate: it lives in
temper-services so temper-api and temper-mcp cannot drift.

| Mode | Rule |
|---|---|
| always | `AUTH_AUDIENCE` present and non-empty |
| always | if `MCP_AUDIENCE` is set, it must **equal** `AUTH_AUDIENCE` |
| `TemperAs` | `AS_AUDIENCE` == `AUTH_AUDIENCE` |
| `TemperAs` | `AS_ISSUER` == `AUTH_ISSUER` |
| `TemperAs` | `JWKS_URL` == `$AS_ISSUER/oauth/jwks` |

**These are not new constraints. They are conditions that already have to hold for the instance to
work at all** — the AS mints one audience, so a divergent `AUTH_AUDIENCE` verifies nothing; a
divergent `AUTH_ISSUER` trusts the wrong issuer; a `JWKS_URL` pointing elsewhere checks no
signature. Any *currently working* AS instance already satisfies all three. We are naming rules that
were already true and failing fast when they aren't — which is why this **cannot take down a working
deployment**. It can only refuse to start one that was already broken and had not noticed.

Two mechanical details:

- **Trailing slashes are normalized before comparison.** Auth0 issuers conventionally end in `/`,
  and `buildAsMetadata` already strips them (`metadata.ts:45`). Comparing raw strings would
  false-positive.
- **Empty string is treated as absent, uniformly.** Today it means "disable" on one surface and
  "reject everything" on the other.

### 3. One parser, one audience

`McpConfig::mcp_audience` is **deleted**. temper-mcp reads the audience off the shared `AuthConfig`.
Keeping both parsers and asserting they agree would be strictly weaker and would leave
`McpConfig`'s no-empty-filter asymmetry alive.

**Be precise about what dies here, because "the fallback" is ambiguous.** The thing being removed is
the *concept of a separate MCP audience* — the notion that this instance has two audiences, one of
which silently substitutes for the other when unset. After this change **an instance has exactly one
audience**, held in `AuthConfig::audience` and used by both surfaces.

The `MCP_AUDIENCE` **env var** survives (removing it would break both live deployments), but it is
demoted to an *agreement assertion*: read in exactly one place, never used as a value, and required
to equal `AUTH_AUDIENCE` if present. An unset `MCP_AUDIENCE` is therefore the **normal, correct**
configuration — not a fallback being exercised. `AUTH_AUDIENCE` is the one source of the one audience.

### 4. Failure behavior — name the variable, prescribe the remedy, print no values

`from_env()` returns a typed `ConfigError` instead of `env::VarError`; `main` / `api/axum.rs` /
`api/mcp.rs` fail the boot.

**Errors name the env var and state the relation it must satisfy. They never print values.** Anyone
who can act on the error (i.e. can mutate env vars) can read those values themselves; the error's
job is to say *which* variable and *what relation*. Guidance is symbolic — `$AS_ISSUER/oauth/jwks`,
not the interpolated URL.

```
AUTH_AUDIENCE is not set. Both surfaces validate the `aud` claim; set it to the API
identifier your IdP mints tokens for.

MCP_AUDIENCE is set but does not equal AUTH_AUDIENCE. This instance validates one audience
on both surfaces. Set them to the same value, or unset MCP_AUDIENCE.

AS_ISSUER is set, so this instance mints its own tokens — but AS_AUDIENCE does not equal
AUTH_AUDIENCE. The authorization server mints every token with AS_AUDIENCE and the API
validates AUTH_AUDIENCE. Set them to the same value.

AS_ISSUER is set, but AUTH_ISSUER does not equal it. The API must trust the authorization
server it fronts. Set AUTH_ISSUER to the same value as AS_ISSUER.

AS_ISSUER is set, but JWKS_URL does not point at this instance's authorization server.
Set JWKS_URL to $AS_ISSUER/oauth/jwks.
```

### 5. Testing

The parser takes an **env lookup** (`impl Fn(&str) -> Option<String>`), not `std::env` directly.
Process env is global and racy under parallel tests; this makes every rule a pure, table-driven unit
test with no `#[serial]` hack, and is the parse-don't-validate shape anyway.

Cases — each a **bite test**, violating exactly one rule and nothing else:

- missing `AUTH_AUDIENCE` → refuse
- empty `AUTH_AUDIENCE` → refuse (the *current* fall-open; this is the security regression test)
- `MCP_AUDIENCE` set and divergent → refuse
- `MCP_AUDIENCE` set and equal → accepted (the current live shape on both deployments)
- `MCP_AUDIENCE` unset → accepted; the instance simply has its one audience
- external-IdP happy path (no `AS_*`) → accepted, `mode == ExternalIdp` — **pins today's prod**
- AS-mode happy path (all agree) → accepted, `mode == TemperAs`
- AS mode, `AS_AUDIENCE` divergent → refuse
- AS mode, `AUTH_ISSUER` divergent → refuse
- AS mode, `JWKS_URL` wrong path → refuse
- trailing-slash normalization on both the issuer and JWKS comparisons → accepted

### 6. Docs

One table in `docs/guides/self-hosting.md`: which vars each mode needs, which must agree, what
breaks when they don't. A pointer from `docs/guides/machine-credentials.md`, which already asks
operators to choose between `provision` and `issue` without saying what config each mode implies.

## Deployment safety

- **Prod (temperkb.io):** external-IdP mode, no `AS_*`, `AUTH_AUDIENCE` verified set and non-empty
  (proved from the cold-start log gap without reading the value — see
  `reference_prove_env_var_nonempty_via_coldstart_log_gap`). `MCP_AUDIENCE` equals it. **Passes.**
- **Enterprise:** both audiences set to one value, confirmed by Cole. **Passes.**
- **Any AS-mode instance:** as argued in §2, a working one already satisfies every rule.

## Out of scope

### Rejected

- **Collapse to a single `TEMPER_AUDIENCE` var.** Conceptually tidier, but breaking for both live
  deployments and a cross-runtime rename (`AS_AUDIENCE` is read from TypeScript) for a coupling this
  design already makes unrepresentable. The rename buys tidiness; the gate buys safety.
- **Keep `MCP_AUDIENCE`'s silent fallback.** It papers over a divergence that does not occur on any
  live deployment, and it is a third rule an operator must hold on top of "these must agree."

### Deferred

- **Validating the remaining `AS_*` surface** (`AS_SIGNING_KEY_PKCS8`, `AS_SIGNING_KID`,
  `AS_CLIENTS`, the TTLs). Real, but they fail loudly at first use rather than falling open — a
  different, lower-severity class. The audience/issuer set is what silently disables a control.
- **Enforcing coherence from the TypeScript side.** The Rust boot gate covers both surfaces; the AS
  itself already `requireEnv`s what it needs.
