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

## The per-surface audience split

This is the one part verification does **not** share, and the reason it stays per-surface
for now:

| | issuer | audience |
|---|--------|----------|
| temper-api | `config.auth_issuer` | `config.auth_audience` |
| temper-mcp | `config.auth_issuer` (same) | `mcp_config.mcp_audience` (**different**) |

Same issuer, different audience — a token minted for the API is not automatically valid at
the MCP endpoint and vice-versa. Both call `jwks_store.validation(issuer, Some(aud), alg)`
with their own audience.

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

- **One issuer per instance.** An instance is either an AS/SAML instance (`AS_ISSUER` set,
  EdDSA) **or** an Auth0/OIDC instance (RS256) — never both. Setting `AS_ISSUER` flips the
  instance into AS mode.
- **AS↔API shared values must agree.** `AS_AUDIENCE == AUTH_AUDIENCE` and
  `AS_ISSUER == AUTH_ISSUER`; `temper admin saml provision` keeps them consistent by
  construction. Details in [../guides/self-hosting-saml.md](../guides/self-hosting-saml.md).
- **On an AS instance, don't diverge `MCP_AUDIENCE`.** temper-mcp resolves its audience as
  `MCP_AUDIENCE ?? AUTH_AUDIENCE` (`temper-mcp/src/config.rs`), while the Temper AS mints
  **every** token — human and machine — with the server-side `AS_AUDIENCE`, ignoring any
  request-supplied `audience` (`mint.ts`). So setting `MCP_AUDIENCE` to something other than
  `AS_AUDIENCE` on an AS instance makes AS-minted tokens unverifiable at the MCP endpoint:
  there is no way to ask that AS for a differently-audienced token. (Auth0-fronted instances
  are unaffected — Auth0 mints to the requested `audience` — and in practice they set the two
  equal anyway.)
