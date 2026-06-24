# Okta Provider Parity — Design

**Date:** 2026-06-24
**Branch:** `jct/okta-provider-parity`
**Status:** Approved design, pre-implementation
**Follows:** `docs/superpowers/specs/2026-06-24-okta-self-hosting-addendum-design.md`

## Context

Writing the Okta self-hosting addendum (PR #164) surfaced two places where Temper assumes the
Auth0 URL shape, each forcing an operator workaround documented in
`docs/guides/self-hosting-okta.md`:

1. **The OIDC `/userinfo` email fallback is Auth0-shaped.** `fetch_email_from_userinfo`
   (`crates/temper-api/src/middleware/auth.rs:189`) builds `{issuer-trimmed}/userinfo`. That
   resolves for Auth0 (`https://tenant.auth0.com/userinfo`) but 404s for an Okta custom
   authorization server, whose userinfo endpoint is `{issuer}/v1/userinfo`. Workaround: Okta
   deployments must put an `email` claim on the access token or login fails.
2. **`temper init` cannot emit Okta URLs.** `provider_and_cloud_sections`
   (`crates/temper-cli/src/commands/init.rs:420-431`) hardcodes Auth0 endpoint shapes
   (`https://{domain}/authorize`, `https://{domain}/oauth/token`) and `provider = "auth0"`.
   Workaround: Okta operators hand-write `config.toml`.

This work removes both workarounds. The two fixes are independent (different crates, no shared
code) and ship together on one branch / PR.

Tracked vault tasks: `019ef9b1-c6c6-73b0-b165-d85f74552bb4` (Task A, userinfo),
`019ef9b1-d409-7092-bbed-e124c201e0cf` (Task B, init).

## Task A — Provider-agnostic `/userinfo` via OIDC discovery

### Decision

Resolve the userinfo endpoint through **OIDC discovery** rather than string-building it from the
issuer. The discovery document URL is the one shape that is consistent across providers —
`{issuer-trimmed}/.well-known/openid-configuration` is correct for Auth0, Okta custom
authorization servers, and Entra — and its `userinfo_endpoint` field is authoritative. (Rejected:
a `USERINFO_URL` env var — another thing to mis-set; and issuer-shape sniffing — brittle
provider-specific string matching.)

### Shape

Two single-purpose, independently testable functions in `crates/temper-api/src/middleware/auth.rs`:

- `discover_userinfo_endpoint(issuer: &str) -> Result<String, String>` — GETs
  `{issuer-trimmed}/.well-known/openid-configuration`, deserializes, returns `userinfo_endpoint`.
- `fetch_email_from_userinfo(userinfo_url: &str, access_token: &str) -> Result<(String, Option<bool>), String>`
  — the existing fetch, now taking a resolved URL instead of an issuer.

A pure helper isolates the parse for unit testing:

- `parse_userinfo_endpoint(body: &str) -> Result<String, String>` — deserializes the discovery
  JSON (a struct with `userinfo_endpoint: Option<String>`) and returns the endpoint or a clear
  error. `discover_userinfo_endpoint` is the network wrapper around it.

### Memoization

The issuer is fixed per deployment, so the resolved endpoint is memoized for the process. Add a
`userinfo_endpoint: tokio::sync::OnceCell<String>` to the middleware's shared application state
(the `state` value that already carries `config`, `jwks_store`, and `pool`; the implementation
confirms its exact type). It is resolved lazily on first use of the fallback path — **not** at boot — so there
is no startup coupling to the IdP being reachable, and the rare fallback path pays the discovery
fetch at most once.

### Error handling

If discovery fails (network error, parse error, or missing `userinfo_endpoint`), return an `Err`
that surfaces as the existing `Unauthorized("Token missing email claim and userinfo lookup
failed")`. **No `{issuer}/userinfo` fallback is retained** — keeping the wrong-shaped URL would
reintroduce the original bug.

### Call-site wiring

The caller in the auth middleware (currently `fetch_email_from_userinfo(&state.config.auth_issuer,
&token)` at auth.rs:105) becomes: resolve the endpoint via the memoized
`discover_userinfo_endpoint`, then call `fetch_email_from_userinfo(endpoint, &token)`.

### Tests

- `parse_userinfo_endpoint` — unit tests over an Auth0-shaped discovery doc
  (`userinfo_endpoint = https://tenant.auth0.com/userinfo`) and an Okta-shaped one
  (`https://org.okta.com/oauth2/<id>/v1/userinfo`); plus a missing-field error case.
- If the crate has HTTP mocking available (e.g. `wiremock`), an integration test that stands up a
  fake discovery + userinfo endpoint and asserts the email round-trips. Otherwise the pure-parse
  tests carry the coverage and the network wrapper stays thin.

## Task B — `temper init` learns Okta URL shapes

### Decision

Teach init to emit Okta's `/oauth2/<authServerId>/v1/*` endpoints. The **written provider label
stays `auth0`** — `provider` / `[[auth.providers]].name` are only the selector that ties
`[auth].provider` to a providers entry (`oauth_config` matches by name,
`crates/temper-client/src/config.rs:54`), not the IdP identity. Keeping the label avoids touching
`temper-client`'s `Provider` enum, the `TEMPER_PROVIDER` override, and stored `auth.json`. A code
comment records why. (Rejected: writing `provider = "okta"` — self-documenting but pulls in the
client auth-storage layer for cosmetic gain.)

### Shape

In `crates/temper-cli/src/commands/init.rs`:

- `enum Idp { Auth0, Okta { auth_server_id: String } }`, carried on `SelfHostConfig`. The variant
  data keeps the invariant that Okta always carries an auth-server-id.
- Pure `fn provider_urls(idp: &Idp, domain: &str) -> (String, String)` returning
  `(authorize_url, token_url)`:
  - `Auth0` → `https://{domain}/authorize`, `https://{domain}/oauth/token`
  - `Okta { auth_server_id }` → `https://{domain}/oauth2/{auth_server_id}/v1/authorize`,
    `https://{domain}/oauth2/{auth_server_id}/v1/token`
- `provider_and_cloud_sections` calls `provider_urls` instead of inlining the Auth0 format strings;
  the `provider`/`name` lines stay `"auth0"`.

### Interactive wizard

After the "self-hosted" branch (init.rs:247), add a `Select` for IdP kind (Auth0 / Okta):

- **Auth0** — unchanged prompts (domain, client_id, audience).
- **Okta** — prompt for Okta org domain (provider-aware hint, e.g. `acme.okta.com`), authorization
  server ID, client_id, audience.

`print_summary` shows `okta (self-hosted)` vs `auth0 (self-hosted)` for operator clarity, even
though the written label is `auth0`.

### Headless flags

In `crates/temper-cli/src/cli.rs`, add to the init command:

- `--idp <auth0|okta>` — defaults to `auth0` (preserves current behavior).
- `--auth-server-id <id>` — **required iff `--idp okta`**; validated with a clear error
  (`--auth-server-id is required when --idp okta`). Ignored/warned for Auth0.

Existing `--instance-url`, `--auth-domain`, `--auth-client-id`, `--auth-audience` are unchanged.

### Tests

- Extend the existing `render_config_toml` tests with an Okta case asserting the
  `/oauth2/<id>/v1/authorize` + `/v1/token` URLs and that `provider`/`name` remain `auth0`.
- Pure `provider_urls` unit test for both variants.
- Headless validation test: `--idp okta` without `--auth-server-id` errors; with it succeeds.

## Shared — documentation + task closure

- Update `docs/guides/self-hosting-okta.md`:
  - "Add an `email` claim to the access token" — downgrade from **required** to **recommended**
    (fast path; userinfo now resolves correctly as the fallback). Adjust the Verify-section
    troubleshooting note accordingly.
  - "Configure the CLI (Okta)" — replace the hand-written-only instruction with the `temper init`
    Okta path (interactive selection + `--idp okta --auth-server-id …` headless), keeping the
    hand-written block as a reference.
  - "Known limitations" — **remove both bullets** (userinfo and init), since both are fixed.
- Close vault tasks A and B (`--stage done`) once merged.

## Out of Scope

### Rejected (load-bearing decisions)
- **Writing `provider = "okta"` into config** — rejected to avoid spreading into `temper-client`'s
  `Provider` enum and auth storage for cosmetic benefit. The label stays `auth0`.
- **`USERINFO_URL` env var / issuer-shape sniffing** — rejected in favor of OIDC discovery.

### Deferred (out of scope here)
- **Server env configuration** — `AUTH_ISSUER`, `JWKS_URL`, `AUTH_AUDIENCE`, `AUTH_PROVIDER_NAME`
  remain operator-set Vercel env vars; init only writes the CLI's `config.toml`.
- **`temper-client` provider modeling** — the `Provider` enum and `TEMPER_PROVIDER` override are
  untouched.

## Execution Note

Two independent streams (temper-api / temper-cli). The implementation plan can run them in
parallel; the PR removes both `self-hosting-okta.md` workarounds together. Consolidated review at
the end of the plan.
