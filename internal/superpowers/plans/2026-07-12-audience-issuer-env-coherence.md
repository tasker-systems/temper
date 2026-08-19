# Audience/Issuer Env Coherence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make an incoherent auth configuration impossible to boot, and make a missing audience impossible to *represent* — closing a fall-open where an unset `AUTH_AUDIENCE` silently disables JWT audience validation on temper-api.

**Architecture:** One typed `AuthConfig` parsed once in `ApiConfig::from_env()` — the choke point both surfaces already call (`api/axum.rs:25`, `api/mcp.rs:24`) — so temper-api and temper-mcp cannot drift. `audience` becomes `String`, not `Option<String>`: deleting the `None` removes the branch `state.rs` uses to set `validate_aud = false`, so the fall-open is unconstructible rather than forbidden. Mode-dependent coherence rules (derived from `AS_ISSUER`'s presence) are enforced at boot.

**Tech Stack:** Rust, `jsonwebtoken` (`Validation`), `tracing`, cargo-make, cargo-nextest.

**Spec:** [`docs/superpowers/specs/2026-07-12-audience-issuer-env-coherence-design.md`](../specs/2026-07-12-audience-issuer-env-coherence-design.md)

## Global Constraints

- All builds and clippy use `--all-features`. Clippy runs with `-D warnings`.
- Use `#[expect(lint, reason = "...")]`, never `#[allow]`.
- All public types implement `Debug`.
- Run `cargo fmt` before every commit (pre-commit hook enforces it, but do it yourself).
- **Errors name the env var and prescribe the relation. They NEVER print values.** Guidance is symbolic (`$AS_ISSUER/oauth/jwks`), never an interpolated URL. Rationale: anyone who can act on the error can read the values themselves.
- Empty string is treated as absent, uniformly, for every variable in this plan.
- Trailing slashes are normalized before any issuer/URL comparison.
- **Out of scope, do not touch:** `crates/temper-cli/src/cli.rs:189` also has an `auth_audience` field. That is the CLI's *client-side* self-host flag (it writes an operator's config file), unrelated to the server's `ApiConfig`. Leave it alone.

---

## File Structure

**Task 1 (additive, compiles alone):**
- Create: `crates/temper-services/src/auth_config.rs` — `AuthConfig`, `AuthMode`, `ConfigError`, `parse_auth_config`. Pure: takes an env *lookup*, never reads `std::env`.
- Modify: `crates/temper-services/src/lib.rs` — declare the module.

**Task 2 (one atomic commit — the workspace does not compile between these edits):**
- Modify: `crates/temper-services/src/config.rs` — `ApiConfig` swaps `auth_issuer` + `auth_audience` + `jwks_url` for one `auth: AuthConfig`; `from_env` returns `ConfigError`.
- Modify: `crates/temper-services/src/state.rs` — `validation()` takes `audience: &str`; **delete** the fall-open branch and the test that asserts it.
- Modify: `crates/temper-services/src/auth/email.rs:57` — `config.auth_issuer` → `config.auth.issuer`.
- Modify: `crates/temper-api/src/middleware/auth.rs:65-67`
- Modify: `crates/temper-mcp/src/config.rs` — **delete** `mcp_audience`.
- Modify: `crates/temper-mcp/src/middleware.rs:55-60` — read the audience off the shared `AuthConfig`.
- Modify boot sites: `crates/temper-api/src/main.rs:19`, `api/axum.rs:25`, `api/mcp.rs:24-25`.
- Modify fixtures: `crates/temper-services/src/auth/mod.rs:226`, `crates/temper-api/tests/common/mod.rs:299,346`, and the six e2e files listed in Task 2 Step 6.

**Task 3:**
- Modify: `docs/guides/self-hosting.md` — the mode/var table.
- Modify: `docs/guides/machine-credentials.md` — a pointer to it.

---

### Task 1: The parser — pure, table-driven, no `std::env`

**Files:**
- Create: `crates/temper-services/src/auth_config.rs`
- Modify: `crates/temper-services/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `auth_config.rs`

**Interfaces:**
- Produces:
  - `pub enum AuthMode { ExternalIdp, TemperAs }` (derives `Debug, Clone, Copy, PartialEq, Eq`)
  - `pub struct AuthConfig { pub issuer: String, pub jwks_url: String, pub audience: String, pub mode: AuthMode }` (derives `Debug, Clone`)
  - `pub enum ConfigError { Missing(&'static str), MissingAudience, McpAudienceMismatch, AsAudienceMismatch, AsIssuerMismatch, AsJwksMismatch }` (derives `Debug, PartialEq, Eq`; impls `Display` + `std::error::Error`)
  - `pub fn parse_auth_config(lookup: impl Fn(&str) -> Option<String>) -> Result<AuthConfig, ConfigError>`

> **Why a lookup closure and not `std::env`:** process env is global and racy under parallel tests. A closure makes every rule a pure unit test with no `#[serial]` hack, and is the parse-don't-validate shape anyway.

> **Why `pub` fields are fine:** the invariant the *type* enforces is "an audience always exists" (`String`, not `Option`). Cross-field coherence is a boot-time concern, not a construction-time one — a test fixture building an `AuthConfig` directly still cannot produce a missing audience, which is the actual vulnerability. Do not add a private constructor; it would buy nothing and make every fixture worse.

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-services/src/auth_config.rs` with ONLY the test module first (the code won't compile yet — that's the point):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a lookup from pairs. Absent keys return None.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    /// A valid external-IdP instance: exactly today's production shape.
    fn external_idp() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AUTH_ISSUER", "https://tenant.auth0.com/"),
            ("JWKS_URL", "https://tenant.auth0.com/.well-known/jwks.json"),
            ("AUTH_AUDIENCE", "https://temperkb.io/api"),
        ]
    }

    /// A valid AS-mode instance: every audience collapses to one value.
    fn temper_as() -> Vec<(&'static str, &'static str)> {
        vec![
            ("AUTH_ISSUER", "https://temper.acme.com"),
            ("JWKS_URL", "https://temper.acme.com/oauth/jwks"),
            ("AUTH_AUDIENCE", "https://temper.acme.com/api"),
            ("AS_ISSUER", "https://temper.acme.com"),
            ("AS_AUDIENCE", "https://temper.acme.com/api"),
        ]
    }

    fn with(base: Vec<(&'static str, &'static str)>, k: &'static str, v: &'static str)
        -> Vec<(&'static str, &'static str)>
    {
        let mut out: Vec<_> = base.into_iter().filter(|(key, _)| *key != k).collect();
        out.push((k, v));
        out
    }

    fn without(base: Vec<(&'static str, &'static str)>, k: &'static str)
        -> Vec<(&'static str, &'static str)>
    {
        base.into_iter().filter(|(key, _)| *key != k).collect()
    }

    // --- the happy paths, which pin the two live deployments ---

    #[test]
    fn external_idp_instance_parses_and_is_external_mode() {
        let cfg = parse_auth_config(env(&external_idp())).expect("valid external-IdP config");
        assert_eq!(cfg.mode, AuthMode::ExternalIdp);
        assert_eq!(cfg.audience, "https://temperkb.io/api");
    }

    #[test]
    fn as_instance_parses_and_is_as_mode() {
        let cfg = parse_auth_config(env(&temper_as())).expect("valid AS config");
        assert_eq!(cfg.mode, AuthMode::TemperAs);
    }

    #[test]
    fn mcp_audience_equal_to_auth_audience_is_accepted() {
        // The current live shape on BOTH deployments.
        let e = with(external_idp(), "MCP_AUDIENCE", "https://temperkb.io/api");
        assert!(parse_auth_config(env(&e)).is_ok());
    }

    #[test]
    fn mcp_audience_unset_is_normal_not_a_fallback() {
        // The instance simply has its one audience. Absence is correct, not degraded.
        let cfg = parse_auth_config(env(&external_idp())).expect("valid");
        assert_eq!(cfg.audience, "https://temperkb.io/api");
    }

    // --- the security regression: this is the bug being closed ---

    #[test]
    fn missing_audience_is_refused() {
        let e = without(external_idp(), "AUTH_AUDIENCE");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::MissingAudience));
    }

    #[test]
    fn empty_audience_is_refused_not_treated_as_disabled() {
        // Today this resolves to None and DISABLES audience validation entirely.
        let e = with(external_idp(), "AUTH_AUDIENCE", "");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::MissingAudience));
    }

    // --- one bite test per rule: each violates exactly that rule and nothing else ---

    #[test]
    fn divergent_mcp_audience_is_refused() {
        let e = with(external_idp(), "MCP_AUDIENCE", "https://other.example/api");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::McpAudienceMismatch));
    }

    #[test]
    fn as_mode_divergent_as_audience_is_refused() {
        let e = with(temper_as(), "AS_AUDIENCE", "https://other.example/api");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::AsAudienceMismatch));
    }

    #[test]
    fn as_mode_divergent_auth_issuer_is_refused() {
        let e = with(temper_as(), "AUTH_ISSUER", "https://tenant.auth0.com/");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::AsIssuerMismatch));
    }

    #[test]
    fn as_mode_jwks_pointing_elsewhere_is_refused() {
        let e = with(temper_as(), "JWKS_URL", "https://tenant.auth0.com/.well-known/jwks.json");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::AsJwksMismatch));
    }

    #[test]
    fn as_mode_requires_as_audience() {
        let e = without(temper_as(), "AS_AUDIENCE");
        assert_eq!(parse_auth_config(env(&e)), Err(ConfigError::Missing("AS_AUDIENCE")));
    }

    // --- normalization ---

    #[test]
    fn trailing_slashes_are_normalized_before_comparison() {
        // AS_ISSUER with a trailing slash, JWKS without, AUTH_ISSUER without: all the same instance.
        let e = with(temper_as(), "AS_ISSUER", "https://temper.acme.com/");
        let cfg = parse_auth_config(env(&e)).expect("trailing slash must not be a mismatch");
        assert_eq!(cfg.mode, AuthMode::TemperAs);
    }

    #[test]
    fn a_jwks_url_with_a_trailing_slash_still_matches() {
        let e = with(temper_as(), "JWKS_URL", "https://temper.acme.com/oauth/jwks/");
        assert!(parse_auth_config(env(&e)).is_ok());
    }

    // --- errors are actionable but leak nothing ---

    #[test]
    fn errors_name_the_variable_and_never_print_values() {
        let e = with(external_idp(), "MCP_AUDIENCE", "https://secret-enterprise.internal/api");
        let err = parse_auth_config(env(&e)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("MCP_AUDIENCE"), "must name the offending var");
        assert!(msg.contains("AUTH_AUDIENCE"), "must name the relation's other side");
        assert!(
            !msg.contains("secret-enterprise.internal"),
            "must NEVER print a config value: {msg}"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p temper-services --lib auth_config 2>&1 | tail -20
```

Expected: compile FAIL — `parse_auth_config`, `AuthConfig`, `AuthMode`, `ConfigError` are not defined.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/temper-services/src/auth_config.rs` (above the test module):

```rust
//! The one place an instance's auth identity is parsed.
//!
//! Four environment variables have to agree — `AUTH_AUDIENCE`, `MCP_AUDIENCE`, `AS_AUDIENCE`,
//! `AS_ISSUER` (plus `AUTH_ISSUER` and `JWKS_URL`, which in AS mode must point at that same AS) —
//! and before this module nothing made them. Worse, `AUTH_AUDIENCE` resolved empty-or-unset to
//! `None`, and `None` disabled JWT audience validation outright.
//!
//! So `audience` here is a `String`, never an `Option<String>`. The fall-open branch has nothing
//! left to branch on: it is *unconstructible*, not merely forbidden.
//!
//! The coherence rules below are **not new policy**. The temper AS mints every token — human and
//! machine — with a single `AS_AUDIENCE`, so a working AS instance already satisfies all of them:
//! a divergent audience verifies nothing, a divergent issuer trusts the wrong party, and a
//! misdirected `JWKS_URL` checks no signature. We name rules that were already true, and fail fast
//! when they are not.

use std::fmt;

/// Which issuer fronts this instance. Derived from `AS_ISSUER`'s presence — that variable being set
/// *is* the AS-mode signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// An external IdP (Auth0) mints tokens; temper is a pure resource server.
    ExternalIdp,
    /// Temper's own authorization server mints tokens.
    TemperAs,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExternalIdp => write!(f, "external-IdP"),
            Self::TemperAs => write!(f, "temper-AS"),
        }
    }
}

/// An instance's verified auth identity. Both surfaces read this same value.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub issuer: String,
    pub jwks_url: String,
    /// The one audience this instance validates, on both surfaces. Never optional.
    pub audience: String,
    pub mode: AuthMode,
}

/// A boot-blocking configuration fault.
///
/// Every message names the offending environment variable and states the relation it must satisfy.
/// **No message ever prints a value** — anyone who can act on the error can already read them, and a
/// config value in a log is a liability with no upside.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Missing(&'static str),
    MissingAudience,
    McpAudienceMismatch,
    AsAudienceMismatch,
    AsIssuerMismatch,
    AsJwksMismatch,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(var) => write!(f, "{var} is not set."),
            Self::MissingAudience => write!(
                f,
                "AUTH_AUDIENCE is not set. Both surfaces validate the `aud` claim; set it to the \
                 API identifier your IdP mints tokens for."
            ),
            Self::McpAudienceMismatch => write!(
                f,
                "MCP_AUDIENCE is set but does not equal AUTH_AUDIENCE. This instance validates one \
                 audience on both surfaces. Set them to the same value, or unset MCP_AUDIENCE."
            ),
            Self::AsAudienceMismatch => write!(
                f,
                "AS_ISSUER is set, so this instance mints its own tokens — but AS_AUDIENCE does not \
                 equal AUTH_AUDIENCE. The authorization server mints every token with AS_AUDIENCE \
                 and the API validates AUTH_AUDIENCE. Set them to the same value."
            ),
            Self::AsIssuerMismatch => write!(
                f,
                "AS_ISSUER is set, but AUTH_ISSUER does not equal it. The API must trust the \
                 authorization server it fronts. Set AUTH_ISSUER to the same value as AS_ISSUER."
            ),
            Self::AsJwksMismatch => write!(
                f,
                "AS_ISSUER is set, but JWKS_URL does not point at this instance's authorization \
                 server. Set JWKS_URL to $AS_ISSUER/oauth/jwks."
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Strip trailing slashes. Auth0 issuers conventionally end in `/` and the AS's own metadata
/// already strips them, so a raw string compare would false-positive.
fn norm(s: &str) -> &str {
    s.trim_end_matches('/')
}

/// Read a variable, treating whitespace-only and empty as absent — uniformly, for every variable.
/// Today an empty `AUTH_AUDIENCE` disables validation on temper-api while an empty `MCP_AUDIENCE`
/// makes temper-mcp reject every token. One typo, two opposite failures. Not any more.
fn get(lookup: &impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    lookup(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Parse and verify an instance's auth identity, or refuse to produce one.
pub fn parse_auth_config(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<AuthConfig, ConfigError> {
    let issuer = get(&lookup, "AUTH_ISSUER").ok_or(ConfigError::Missing("AUTH_ISSUER"))?;
    let jwks_url = get(&lookup, "JWKS_URL").ok_or(ConfigError::Missing("JWKS_URL"))?;
    let audience = get(&lookup, "AUTH_AUDIENCE").ok_or(ConfigError::MissingAudience)?;

    // MCP_AUDIENCE is no longer a *value* — it is an agreement assertion. An instance has exactly
    // one audience; this variable may only restate it.
    if let Some(mcp_audience) = get(&lookup, "MCP_AUDIENCE") {
        if mcp_audience != audience {
            return Err(ConfigError::McpAudienceMismatch);
        }
    }

    let mode = match get(&lookup, "AS_ISSUER") {
        None => AuthMode::ExternalIdp,
        Some(as_issuer) => {
            let as_audience =
                get(&lookup, "AS_AUDIENCE").ok_or(ConfigError::Missing("AS_AUDIENCE"))?;
            if as_audience != audience {
                return Err(ConfigError::AsAudienceMismatch);
            }
            if norm(&as_issuer) != norm(&issuer) {
                return Err(ConfigError::AsIssuerMismatch);
            }
            if norm(&jwks_url) != format!("{}/oauth/jwks", norm(&as_issuer)) {
                return Err(ConfigError::AsJwksMismatch);
            }
            AuthMode::TemperAs
        }
    };

    Ok(AuthConfig {
        issuer,
        jwks_url,
        audience,
        mode,
    })
}
```

- [ ] **Step 4: Declare the module**

In `crates/temper-services/src/lib.rs`, add after line 10 (`pub mod auth;`), keeping the list alphabetical:

```rust
pub mod auth_config;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p temper-services --lib auth_config 2>&1 | tail -5
```

Expected: `test result: ok. 13 passed; 0 failed`.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/temper-services/src/auth_config.rs crates/temper-services/src/lib.rs
git commit -m "feat(config): one parser for an instance's auth identity

Four env vars must agree and nothing made them. AUTH_AUDIENCE resolved
empty-or-unset to None, and None disabled JWT audience validation outright.
Here audience is a String, never an Option: the fall-open branch has nothing
left to branch on.

The coherence rules are not new policy. The AS mints every token with a single
AS_AUDIENCE, so a working AS instance already satisfies all of them. We name
rules that were already true and fail fast when they are not.

Additive: nothing consumes this yet."
```

---

### Task 2: The atomic swap — wire it in, delete the fall-open

**Files:**
- Modify: `crates/temper-services/src/config.rs`
- Modify: `crates/temper-services/src/state.rs`
- Modify: `crates/temper-services/src/auth/email.rs:57`
- Modify: `crates/temper-services/src/auth/mod.rs` (test fixture ~line 226)
- Modify: `crates/temper-api/src/middleware/auth.rs:65-67`
- Modify: `crates/temper-api/src/main.rs:19`
- Modify: `crates/temper-mcp/src/config.rs`
- Modify: `crates/temper-mcp/src/middleware.rs:55-60`
- Modify: `api/axum.rs:25`, `api/mcp.rs:24-25`
- Modify: `crates/temper-api/tests/common/mod.rs` (~lines 299, 346)
- Modify (e2e fixtures): `tests/e2e/tests/common/mod.rs`, `tests/e2e/tests/auth_seam_m2m_e2e.rs`, `tests/e2e/tests/auth_seam_parity_e2e.rs`, `tests/e2e/tests/act_authorship_mcp_e2e.rs`, `tests/e2e/tests/mcp_round_trip_test.rs`, `tests/e2e/tests/mcp_segmented_ingest_test.rs`

**Interfaces:**
- Consumes from Task 1: `AuthConfig`, `AuthMode`, `ConfigError`, `parse_auth_config`.
- Produces: `ApiConfig { pub auth: AuthConfig, .. }` — the fields `auth_issuer`, `auth_audience`, and `jwks_url` **no longer exist**; read `config.auth.issuer`, `config.auth.audience`, `config.auth.jwks_url`. `JwksKeyStore::validation(&self, issuer: &str, audience: &str, algorithm: Algorithm) -> Validation` — `audience` is no longer `Option`.

> **Why this is ONE commit and not five.** Removing `Option` from a struct field breaks every constructor in the workspace simultaneously — `ApiConfig` has struct literals in nine files across four crates plus the e2e suite. Rust will not compile in between. Splitting this would mean committing a broken tree. Cross-crate type refactors land atomically.

- [ ] **Step 1: Retype `ApiConfig` and make `from_env` fallible**

Rewrite `crates/temper-services/src/config.rs`:

```rust
use crate::auth_config::{parse_auth_config, AuthConfig, ConfigError};
use std::env;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub database_url: String,
    /// This instance's verified auth identity — issuer, JWKS, the one audience, and the mode.
    /// Replaces the old `auth_issuer` / `auth_audience` / `jwks_url` trio, whose `Option<String>`
    /// audience was what let audience validation silently switch itself off.
    pub auth: AuthConfig,
    pub auth_provider_name: String,
    pub cors_origins: Vec<String>,
    pub port: u16,
    pub enable_swagger: bool,
    /// Shared secret gating the internal SAML reconcile endpoint. `None` disables the endpoint.
    pub internal_reconcile_secret: Option<String>,
    /// Shared secret gating the internal embed-dispatch drain endpoint (issue #299), called by the
    /// Vercel cron. `None` disables the endpoint (a deployment with no drain configured).
    pub embed_dispatch_secret: Option<String>,
}

impl ApiConfig {
    /// Load from the process environment. Refuses to produce a config an instance cannot serve on.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Load from an arbitrary lookup. Exists so config is testable without touching the global,
    /// racy process environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let auth = parse_auth_config(&lookup)?;

        // The mode is not consulted after parsing. It is logged because an operator who cannot tell
        // which mode their instance is in is exactly the operator who mis-sets these variables.
        tracing::info!(mode = %auth.mode, "auth configured");

        let cors_origins: Vec<String> = lookup("CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if cors_origins.is_empty() {
            tracing::info!(
                "CORS_ORIGINS is not set — cross-origin requests will be denied. \
                 Set CORS_ORIGINS=* for permissive mode in development."
            );
        }

        let enable_swagger = lookup("ENABLE_SWAGGER")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        if enable_swagger {
            tracing::info!("Swagger UI enabled at /api-docs/ui");
        }

        Ok(Self {
            database_url: lookup("DATABASE_URL").ok_or(ConfigError::Missing("DATABASE_URL"))?,
            auth,
            auth_provider_name: lookup("AUTH_PROVIDER_NAME")
                .unwrap_or_else(|| "auth0".to_string()),
            cors_origins,
            port: lookup("PORT").and_then(|p| p.parse().ok()).unwrap_or(3000),
            enable_swagger,
            internal_reconcile_secret: lookup("INTERNAL_RECONCILE_SECRET").filter(|s| !s.is_empty()),
            embed_dispatch_secret: lookup("EMBED_DISPATCH_SECRET").filter(|s| !s.is_empty()),
        })
    }
}
```

- [ ] **Step 2: Delete the fall-open in the validator, and the test that guards it**

In `crates/temper-services/src/state.rs`, change `validation` (~line 139) so `audience` is not optional:

```rust
    pub fn validation(&self, issuer: &str, audience: &str, algorithm: Algorithm) -> Validation {
        let mut v = Validation::new(algorithm);
        v.algorithms = vec![algorithm];
        v.set_issuer(&[issuer]);
        v.set_audience(&[audience]);
        v
    }
```

The old `else { v.validate_aud = false; }` branch is **deleted**. There is no longer an input that reaches it.

**Delete the test `validation_without_audience_disables_aud_check` (~line 256) outright.** It asserts that a missing audience disables the `aud` check — it pins the vulnerability as correct behavior. Do not adapt it; remove it.

Update the two surviving `validation(...)` callers in that file's tests (~lines 276 and 395) to pass a plain `&str` instead of `Some("...")`.

- [ ] **Step 3: Update the two middlewares to read the shared audience**

`crates/temper-api/src/middleware/auth.rs`, ~lines 65-67:

```rust
    let issuer = &state.config.auth.issuer;
    let audience = state.config.auth.audience.as_str();
    let validation = state.jwks_store.validation(issuer, audience, vk.algorithm);
```

`crates/temper-mcp/src/middleware.rs`, ~lines 55-60:

```rust
    let issuer = &state.api_state.config.auth.issuer;
    let audience = state.api_state.config.auth.audience.as_str();
    // ...
        .validation(issuer, audience, vk.algorithm);
```

`crates/temper-services/src/auth/email.rs:57`: `&state.config.auth_issuer` → `&state.config.auth.issuer`.

- [ ] **Step 4: Delete `McpConfig::mcp_audience`**

In `crates/temper-mcp/src/config.rs`, remove the `mcp_audience` field from the struct **and** its `env::var("MCP_AUDIENCE").or_else(...)` read in `from_env`. temper-mcp now reads the audience off the shared `AuthConfig` (Step 3).

What dies here is *the concept of a second, MCP-specific audience*. The `MCP_AUDIENCE` **env var** still exists and is still honored — but only in `parse_auth_config`, and only as an assertion that it equals `AUTH_AUDIENCE`. An unset `MCP_AUDIENCE` is the normal, correct configuration, not a fallback being exercised.

- [ ] **Step 5: Fail the boot, loudly**

`crates/temper-api/src/main.rs:19`, `api/axum.rs:25`, `api/mcp.rs:24`:

```rust
    let config = ApiConfig::from_env()
        .unwrap_or_else(|e| panic!("refusing to start: {e}"));
```

`api/mcp.rs` keeps its separate `McpConfig::from_env()` call (that struct still exists for `MCP_BASE_URL` / `MCP_CLIENT_ID` / the TOML) — only its audience field is gone.

> A `warn!` was never a control. An instance that cannot state which audience it validates must not serve traffic.

- [ ] **Step 6: Update every fixture**

Each `ApiConfig` struct literal replaces its `jwks_url` / `auth_issuer` / `auth_audience: None` fields with a single `auth:` field. Add `use temper_services::auth_config::{AuthConfig, AuthMode};` (or `crate::auth_config::...` inside temper-services).

```rust
        auth: AuthConfig {
            issuer: "test-issuer".to_string(),
            jwks_url: "unused".to_string(),
            audience: "test-audience".to_string(),
            mode: AuthMode::ExternalIdp,
        },
```

**Important:** fixtures previously passed `auth_audience: None`, which meant tests ran with audience validation *off*. Now they assert an audience. Any test JWT those fixtures verify must carry a matching `aud` claim, or it will now (correctly) fail. **Expect to add an `aud` to the test-JWT builders** — in `tests/e2e/tests/common/mod.rs` look at `generate_test_jwt` and `generate_machine_jwt`, and in `crates/temper-api/tests/common/mod.rs` the equivalent. Set the claim to the same `"test-audience"` string the fixture uses. This is not incidental churn: it is the tests finally exercising the control that production relies on.

Files: `crates/temper-services/src/auth/mod.rs` (~226), `crates/temper-api/tests/common/mod.rs` (~299, ~346), `tests/e2e/tests/common/mod.rs`, `tests/e2e/tests/auth_seam_m2m_e2e.rs`, `tests/e2e/tests/auth_seam_parity_e2e.rs`, `tests/e2e/tests/act_authorship_mcp_e2e.rs`, `tests/e2e/tests/mcp_round_trip_test.rs`, `tests/e2e/tests/mcp_segmented_ingest_test.rs`.

- [ ] **Step 7: Compile the whole workspace**

```bash
cargo make check > /tmp/check.log 2>&1; echo "exit=$?"; tail -20 /tmp/check.log
```

Expected: `exit=0`. If a struct literal was missed, the compiler names the file and field — fix and re-run.

- [ ] **Step 8: Run the unit and DB test tiers**

```bash
cargo make test 2>&1 | tail -5
cargo make test-db 2>&1 | tail -5
```

Expected: green. A failure here is most likely a test JWT missing its `aud` claim (see Step 6) — that is the control working, not a regression.

- [ ] **Step 9: Run e2e — this tier is the one that matters**

The e2e suite drives real JWT verification through both surfaces, so it is what actually proves the audience gate is live and that neither surface fell over.

```bash
cargo build -p temper-cli --bin temper   # nextest does NOT rebuild the spawned binary
cargo make test-e2e 2>&1 | tail -10
```

Expected: green.

- [ ] **Step 10: Commit**

```bash
cargo fmt
git add -A
git commit -m "fix(config): fail closed on an incoherent auth configuration

ApiConfig::auth_audience was an Option<String>, and None reached
JwksKeyStore::validation, which set validate_aud = false — an unset or empty
AUTH_AUDIENCE silently disabled JWT audience validation on temper-api behind
nothing but a tracing::warn. temper-mcp always enforced. That is the
surface-asymmetry class #384/#388 closed everywhere else, surviving in
configuration rather than in code.

The audience is now a String on a typed AuthConfig, parsed once at the choke
point both surfaces already call. The fall-open branch is deleted because
nothing can reach it — unconstructible, not forbidden. The test that asserted
validate_aud = false is deleted too: it pinned the vulnerability as correct.

McpConfig::mcp_audience is gone. An instance has exactly one audience; the
MCP_AUDIENCE env var survives only as an assertion that it restates it.

Atomic by necessity: removing Option from the field breaks every ApiConfig
constructor in the workspace at once.

Task: 019f5623-0ed2."
```

---

### Task 3: The operator's table

**Files:**
- Modify: `docs/guides/self-hosting.md`
- Modify: `docs/guides/machine-credentials.md`

**Interfaces:**
- Consumes: the rules as implemented in `parse_auth_config` (Task 1). Read that function before writing — the doc must describe what the code does, not what this plan says it does.

- [ ] **Step 1: Add the mode/var table to `docs/guides/self-hosting.md`**

Match the guides' house style: no front-matter, `**Audience:**` / `**Scope:**` lead-ins, tables for matrices, blockquotes for the load-bearing gotcha, bash blocks with a `#` comment above each command, `## See also` at the end with relative links.

The table must answer, for each mode, which variables are required and which must agree:

| Variable | External IdP (Auth0) | Temper AS (`AS_ISSUER` set) |
|---|---|---|
| `AUTH_ISSUER` | your IdP's issuer | **must equal `AS_ISSUER`** |
| `JWKS_URL` | your IdP's JWKS | **must be `$AS_ISSUER/oauth/jwks`** |
| `AUTH_AUDIENCE` | **required** — the API identifier | **required** — must equal `AS_AUDIENCE` |
| `MCP_AUDIENCE` | optional; if set **must equal `AUTH_AUDIENCE`** | same |
| `AS_ISSUER` | unset | **set — this is what selects AS mode** |
| `AS_AUDIENCE` | unset | **required** — must equal `AUTH_AUDIENCE` |

State plainly, in a blockquote, the fact that makes it all cohere:

> The temper AS mints **every** token — human and machine — with a single `AS_AUDIENCE`. So on an AS
> instance the three audiences are **one value**. They are not three knobs; they are one knob spelled
> three ways, and temper now refuses to start if they disagree.

And the consequence an operator actually needs:

> An incoherent configuration **fails the boot**, naming the variable and the relation it must
> satisfy. This is deliberate: a warning in a serverless log is not a control.

- [ ] **Step 2: Point at it from `docs/guides/machine-credentials.md`**

That guide already makes operators choose between `provision` and `issue` without saying what config each mode implies. Add a line in its `## See also` linking the new table, e.g.:

```markdown
- [Self-Hosting](self-hosting.md#authentication-environment) — which auth variables each mode needs, and which must agree.
```

(Use the real anchor for the section you added in Step 1.)

- [ ] **Step 3: Verify no other doc now contradicts the code**

```bash
grep -rn "AUTH_AUDIENCE\|MCP_AUDIENCE\|AS_AUDIENCE" docs/ --include='*.md' | grep -v superpowers/
```

Read each hit. Anything that says audience validation is optional, or that `MCP_AUDIENCE` defaults/falls back to `AUTH_AUDIENCE`, is now false — fix it.

- [ ] **Step 4: Commit**

```bash
git add docs/guides/
git commit -m "docs(guides): one table for the auth env vars each mode needs

An operator previously had to hold four variables together with no doc saying
they were coupled, and no doc saying the coupling is mode-dependent. The AS
mints every token with one AS_AUDIENCE, so on an AS instance the three
audiences are one value spelled three ways.

Task: 019f5623-0ed2."
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| §1 fall-open unconstructible (`audience: String`) | Task 1 (type), Task 2 Step 2 (delete the branch) |
| §2 the five rules + normalization + empty-as-absent | Task 1 Step 3 |
| §3 one parser; `McpConfig::mcp_audience` deleted | Task 2 Steps 3–4 |
| §4 typed `ConfigError`; boot fails; names not values | Task 1 Step 3 (`Display`), Task 2 Step 5 (boot) |
| §5 env-lookup parser; one bite test per rule | Task 1 Steps 1–3 |
| §6 docs table + pointer | Task 3 |
| Deployment safety (both live shapes pass) | Task 1 tests `external_idp_instance_parses…` and `mcp_audience_equal_to_auth_audience_is_accepted` pin exactly the prod/enterprise shape |

**Type consistency:** `AuthConfig` fields (`issuer`, `jwks_url`, `audience`, `mode`) are used identically in Task 1's definition, Task 2's fixtures, and both middlewares. `validation(issuer: &str, audience: &str, algorithm)` matches its two production callers and its remaining tests. `ConfigError` variant names match between `Display`, the parser, and the Task 1 assertions.

**Known follow-on (not a gap):** Task 2 Step 6 will require adding an `aud` claim to the test-JWT builders, because fixtures previously ran with audience validation *off*. Called out inline rather than discovered mid-task.
