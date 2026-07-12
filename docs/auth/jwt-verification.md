# JWT verification

Both surfaces verify a Bearer JWT before anything reaches the [authorization
seam](./authorization-seam.md). Verification stays **per-surface** — and it is now the *only*
thing that does, because the audience differs legitimately. The shared machinery is the
`JwksKeyStore`; everything downstream of the decode (classification, the email ladder, claim
construction, the gates) is the seam's.

Source: `crates/temper-services/src/state.rs` (`JwksKeyStore`),
`crates/temper-api/src/middleware/auth.rs`, `crates/temper-mcp/src/middleware.rs`.

## Two issuers, one verifier

An instance validates tokens from exactly **one** issuer, configured by env:

- **Auth0 / OIDC** (temperkb.io and Okta-fronted self-hosting). Tokens are **RS256**.
  `JwksKeyStore` fetches the RSA public key from Auth0's JWKS.
- **Temper Authorization Server** (native SAML self-hosting). The AS mints **EdDSA**
  (Ed25519) tokens; `JwksKeyStore` fetches the OKP key from the AS's `/oauth/jwks`.
  See [../guides/self-hosting-saml.md](../guides/self-hosting-saml.md) for how the AS is
  stood up.

Both issuers mint **human and machine** tokens, with the same signing key per issuer — a
`client_credentials` token is not a separate key family, only a separate *claim shape*. So
verification is identical for both and the split happens one step later, in the seam's
classifier.

`JwksKeyStore` supports both key families and maps each to its algorithm:

```rust
// state.rs::algorithm_for_key
RSA               → RS256
OKP / Ed25519     → EdDSA
```

**Single-family validation allow-list.** `jsonwebtoken`'s `verify_signature` rejects any
`Validation` whose algorithm allow-list contains a family the loaded key does not match. So
the algorithm must travel *with* the key: `get_decoding_key()` returns a `VerificationKey {
key, algorithm }`, and `validation(issuer, audience, algorithm)` scopes the allow-list to
exactly that one algorithm. This is why the store returns the algorithm rather than letting
the caller guess it.

The JWKS is cached with a 1-hour TTL (`JwksKeyStore::new`); tests preload a static key via
`with_static_key`.

## One audience, both surfaces

There used to be a **per-surface audience split** here: temper-api validated `config.auth_audience`
while temper-mcp validated `mcp_config.mcp_audience`, parsed separately from `MCP_AUDIENCE` with a
fallback to `AUTH_AUDIENCE`. That is gone.

| | issuer | audience |
|---|--------|----------|
| temper-api | `config.auth.issuer` | `config.auth.audience` |
| temper-mcp | `config.auth.issuer` (same) | `config.auth.audience` (**the same**) |

Both call `jwks_store.validation(issuer, audience, alg)` — note `audience: &str`, not
`Option<&str>`. An instance has exactly **one** audience, parsed once at boot.

Two parsers for one concept is precisely how the surfaces came to disagree: an empty
`AUTH_AUDIENCE` made temper-api set `validate_aud = false` and accept everything, while an empty
`MCP_AUDIENCE` made temper-mcp enforce `aud == ""` and reject everything. One typo, two opposite
failures, neither of them anyone's decision.

> **`set_audience` is not sufficient on its own.** `jsonwebtoken` only *compares* the audience when
> the `aud` claim is **present** — `required_spec_claims` defaults to `{"exp"}`. A token omitting
> `aud` entirely was accepted even with `validate_aud = true`. `validation()` therefore sets
> `required_spec_claims(&["exp", "iss", "aud"])`. Requiring the *value* to match without requiring
> the *claim* to exist closes half a door.

## What the surface hands the seam

Verification produces raw JWT claims. The surface decodes them into the shared
`temper_services::auth::RawJwtClaims` — a *superset* struct whose optional fields (`email`,
`email_verified`, `azp`, `gty`) absorb the human/machine shape difference — and hands the seam
**two** things:

| | |
|---|---|
| `RawJwtClaims` | the decoded claims, exactly as verified |
| the raw bearer `&str` | needed by one rung of the email ladder — the `/userinfo` call presents the token itself, not its claims |

That is the whole handoff: `authenticate_token(&state, &raw, token)`. The surface **does not**
build an `AuthClaims` — the seam is the only constructor. On temper-mcp the two values travel
through the HTTP extensions as `RawJwtClaims` + `BearerToken` (a newtype, so it cannot be
confused with any other string in the extensions map).

## The email-resolution ladder (in the seam, for both surfaces)

`crates/temper-services/src/auth/email.rs`. It used to live in temper-api's middleware, and
*only* there — temper-mcp set `email: String::new()` and auto-provisioned. It now runs for
whichever surface presented the token, and resolves in order:

1. the `email` claim embedded in the token (a custom Auth0 Action can add it), else
2. a previously cached email in `kb_profile_auth_links` (from a prior login), else
3. the OIDC `/userinfo` endpoint (discovered once per process via
   `/.well-known/openid-configuration`) as a last resort.

Falling off the bottom is `AuthzError::EmailResolution` → a `401` (HTTP) / a terminal
`INVALID_REQUEST` (MCP): **a human we cannot name is a human we will not provision.** The
ladder is deliberately concrete rather than a trait — there is exactly one implementation and
no policy to vary, and a surface that wanted a different email answer would be re-introducing
the drift.

The ladder is on the **human arm only**. A machine token has no human email and no
`/userinfo` to ask, so running it on that path would be an authentication failure dressed as a
lookup — see the [machine-token contract](./machine-token-contract.md).

## Instance-mode invariants

These are no longer advice. They are **enforced at boot** by `parse_auth_config`
(`temper-services/src/auth_config.rs`) — an instance that violates any of them refuses to start,
naming the variable and the relation it must satisfy. They used to be operator discipline, which is
how they came to be violated silently.

- **One issuer per instance.** An instance is either an AS/SAML instance (`AS_ISSUER` set,
  EdDSA) **or** an Auth0/OIDC instance (RS256) — never both. Setting `AS_ISSUER` flips the
  instance into AS mode (`AuthMode::TemperAs`).
- **`AUTH_AUDIENCE` is mandatory.** Empty counts as unset. It used to resolve to `None`, which set
  `validate_aud = false` and **disabled audience validation outright**; there is no longer an
  `Option` to carry that state.
- **AS↔API shared values must agree.** `AS_AUDIENCE == AUTH_AUDIENCE`, `AS_ISSUER == AUTH_ISSUER`,
  and `JWKS_URL == $AS_ISSUER/oauth/jwks` (trailing slashes normalized before comparison).
  `temper admin saml provision` keeps them consistent by construction. Details in
  [../guides/self-hosting-saml.md](../guides/self-hosting-saml.md).
- **An instance has exactly ONE audience.** temper-mcp no longer carries its own — the
  `mcp_audience` field is gone, and both surfaces read `AuthConfig::audience`. `MCP_AUDIENCE`
  survives as an env var, but purely as an assertion: if set, it **must equal** `AUTH_AUDIENCE`, or
  the instance does not boot. There was previously an `MCP_AUDIENCE ?? AUTH_AUDIENCE` fallback, and
  two parsers for one concept is precisely how the surfaces came to answer an empty value in
  opposite ways — temper-api fell open, temper-mcp fell shut.

  This matters most on an AS instance: the Temper AS mints **every** token — human and machine —
  with the server-side `AS_AUDIENCE`, ignoring any request-supplied `audience` (`mint.ts`). There is
  no way to ask it for a differently-audienced token, so a divergent `MCP_AUDIENCE` would make
  AS-minted tokens unverifiable at `/mcp`. That is now unreachable rather than merely discouraged.
