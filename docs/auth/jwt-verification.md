# JWT verification

Both surfaces verify a Bearer JWT before anything reaches the [authorization
seam](./authorization-seam.md). Verification stays **per-surface** for the human-token cut
because the audience differs legitimately; the shared machinery is the `JwksKeyStore`.
(Verification moves *into* the seam only when machine tokens arrive — see the
[machine-token contract](./machine-token-contract.md).)

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

## Claim normalization → `AuthClaims`

Verification produces raw JWT claims; each surface normalizes them into the `AuthClaims`
the seam consumes.

**temper-api — the email-resolution ladder.** `AuthClaims.email` is resolved in order
(`resolve_email_from_claims`):

1. the `email` claim embedded in the token (a custom Auth0 Action can add it), else
2. a previously cached email in `kb_profile_auth_links` (from a prior login), else
3. the OIDC `/userinfo` endpoint (discovered via `/.well-known/openid-configuration`) as a
   last resort. Failure here is a `401` — the token is missing email and we could not
   recover it.

**temper-mcp — the empty-email path.** MCP tokens may omit email; `claims_from` sets
`email: String::new()` and lets `resolve_from_claims` recover it from the cached auth link
downstream (`service.rs`). A machine token has no human email at all — see the
[machine-token contract](./machine-token-contract.md).

Normalization is **input** to the seam. The seam does not verify signatures or resolve
email; it takes an `AuthClaims` and owns resolve + the gates.

## Instance-mode invariants

- **One issuer per instance.** An instance is either an AS/SAML instance (`AS_ISSUER` set,
  EdDSA) **or** an Auth0/OIDC instance (RS256) — never both. Setting `AS_ISSUER` flips the
  instance into AS mode.
- **AS↔API shared values must agree.** `AS_AUDIENCE == AUTH_AUDIENCE` and
  `AS_ISSUER == AUTH_ISSUER`; `temper admin saml provision` keeps them consistent by
  construction. Details in [../guides/self-hosting-saml.md](../guides/self-hosting-saml.md).
</content>
