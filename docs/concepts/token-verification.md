# Token Verification

**For operators and integrators** — anyone who needs to understand how Temper verifies the
tokens it receives. Operators configure the issuer; integrators need to know what the server
checks.

## Two issuers, one verifier

An instance validates tokens from exactly **one** issuer:

- **Auth0 / OIDC** (the hosted instance and Okta-fronted self-hosting). Tokens are RS256; the
  public key is fetched from the issuer's JWKS endpoint.
- **Temper Authorization Server** (native SAML self-hosting). Tokens are EdDSA (Ed25519); the
  key is fetched from the AS's own JWKS endpoint.

Both issuers mint **human and machine** tokens with the same signing key per issuer — a
`client_credentials` token is not a separate key family, only a separate claim shape.
Verification is identical for both; the split happens one step later, in classification.

## One audience check, both surfaces

Both surfaces run the same audience check, and the check reads a **set**, not a single value:
the auth audience, plus — on the MCP surface — its own RFC 8707 resource audience when one is
configured. A token naming either audience verifies; a token naming neither does not.

This is enforced: the audience is a mandatory field (empty counts as unset), and the JWT must
carry the `aud` claim — requiring the value to match without requiring the claim to exist
would close only half the door. The server also requires `exp` (expiry) and `iss` (issuer).

What each audience *is* — the auth audience, the optional MCP audience, the AS audience, and
which of them must agree — is the auth-identity contract's to define, not this page's:
[Auth identity](./auth-identity.md).

## What the surface hands the seam

Verification produces raw JWT claims. The surface decodes them and hands the authorization seam
two things: the decoded claims and the raw bearer token. The surface does not build a
principal — the seam is the only constructor. This is so a surface cannot construct a
*different* principal than its sibling would, which is exactly how the two surfaces
historically drifted.

## The email-resolution ladder

For human tokens, the seam resolves the caller's email in order:

1. The `email` claim embedded in the token (if the issuer adds one), else
2. A previously cached email from a prior login, else
3. The OIDC `/userinfo` endpoint as a last resort.

Falling off the bottom is a 401: **a human we cannot name is a human we will not provision.**
The ladder is on the human arm only — a machine token has no email and no `/userinfo` to ask.

## Instance-mode invariants

These are enforced at boot, not operator discipline:

- **One issuer per instance.** Setting the AS issuer flips the instance into AS mode.
- **Audiences are instance config, read as a set.** The auth audience is mandatory; the
  optional MCP audience is per-instance, never per-request — the model is
  [Auth identity](./auth-identity.md)'s.
- **AS↔API shared values must agree.** If the AS audience, auth audience, or JWKS URL diverge,
  the instance refuses to start.

## Further reading

- **The trust boundary this verification feeds into:**
  [The Trust Boundary](./trust-boundary.md).
- **The auth-identity contract operators configure:**
  [Auth identity](./auth-identity.md).
- **Machine tokens and their claim shape:**
  [Machine tokens](./machine-tokens.md).
- **What the architecture fixes vs. what a deployment chooses:**
  [temperkb.io/operating/deployment](https://temperkb.io/operating/deployment).
