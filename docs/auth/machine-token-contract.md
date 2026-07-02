# The issuer / resource-server boundary & the machine-token contract

This document is the **canonical home** for the single machine-token claim contract that
Temper's two token issuers conform to and the Rust seam normalizes. It is the unifying
artifact for M2M (machine-to-machine) agent principals — the auth-seam plan's Stage 4.

> **Status:** the contract is **specified here; the code that consumes it is Stage 4, not
> yet built.** Human tokens flow today. This document is written first, deliberately, so the
> AS-mint path and the Rust normalizer are built *to match it* rather than drifting into two
> shapes.

## The boundary: who mints vs. who validates

Temper's token boundary is **issuer-mints / resource-server-validates**, and it stays split:

- **Issuers (TypeScript or Auth0).** The OAuth Authorization Server is entirely TypeScript
  (`packages/temper-cloud`) or Auth0. Grant advertisement is single-sourced in TS
  (`src/oauth/metadata.ts`), which branches on `AS_ISSUER`:
  - **Auth0-fronted instance** (temperkb.io): `token_endpoint` points at Auth0; Auth0 mints.
    `grant_types_supported` is advertised by `buildAuth0AsMetadata`.
  - **Temper AS instance** (self-hosted SAML): `token_endpoint` → temper-cloud's own
    `handleToken`, which mints EdDSA tokens via `mintAccessToken`.
- **Resource server (Rust).** `temper-api` and `temper-mcp` are pure resource servers:
  they **validate and normalize** tokens. Rust **never advertises or mints**.

So there is **no cross-language advertisement split to unify** — advertisement already lives
only in TS. The only thing worth pinning across the boundary is the **token claim shape**.
That is this contract.

## The single machine-token claim shape

A machine token (an agent acting *as itself*, no human) carries a distinct claim shape that
**both** issuers must produce identically, and the Rust normalizer parses as exactly one
machine shape regardless of issuer:

| Claim | Value | Note |
|-------|-------|------|
| `azp` / `client_id` | `<clientid>` | the stable agent identity; key the agent profile on this |
| `sub` | `<clientid>@clients` | Auth0's M2M convention |
| `gty` | `client-credentials` | grant-type marker |
| `email` | *(absent)* | a machine has no verified human email |
| `aud` | the target API/MCP audience | passed in by the validating surface |

The normalizer **detects** this shape (machine vs. human), stamps a distinct provider tag
(e.g. `auth0-m2m`) into `AuthClaims`, and `resolve_from_claims` **branches on that tag**:

- **Human provider** → the existing email-reconcile path
  ([jwt-verification.md](./jwt-verification.md) ladder).
- **Machine provider** → a `(provider, client_id)` link lookup; on first sight, provision a
  **dedicated agent profile** (a `kb_profile_auth_links` row under the machine provider). It
  **never** enters `reconcile_by_email` — there is no verified email.

> **Open detail to lock in Stage 4:** prefer reading `azp` directly over stripping the
> `@clients` suffix from `sub`; validate against a real Auth0 M2M token when the app is
> provisioned. The `auth0-m2m` tag value is a naming choice to settle then.

## Agent principals ride the ordinary rails

Once provisioned, an agent profile is an ordinary accountable principal — no auth-path
special-casing:

- It passes `is_active` and `system_access` on the **same rails** as a human (see the
  [two-level chain](./authorization-seam.md)).
- It takes **ordinary grants**: team membership for source read, `cogmap grant --write` for
  authoring. It is its own accountable principal (fits the invocation-envelope model), never
  a proxied human.

## Where verification enters the seam

Stage 4b is where JWT verification finally moves **into** the seam (it stays per-surface for
human tokens today — see [jwt-verification.md](./jwt-verification.md)). The reason is
exactly the machine shape: a shared normalizer must own JWKS decode + the human/machine
branch, with audience passed in by the caller. Verification centralizes when M2M claim-shape
divergence makes it pay for itself — not speculatively.

## Stage-4 delivery split (for reference)

- **4a — Auth0 branch (unblocks the steward).** Add `client_credentials` to
  `grant_types_supported` in `buildAuth0AsMetadata` (one line, TS) + provision an Auth0 M2M
  application for the API audience. Auth0 mints; no token-minting code in the repo.
- **4b — Rust resource-server side.** The shared machine-claim normalizer + provider-branched
  agent-profile provisioning described above.
- **4c — Temper AS branch (deferrable).** `handleToken` implements the `client_credentials`
  grant and mints via a **machine variant of `MintedClaims`** matching this contract — ideally
  a ts-rs-shared machine-claim type in temper-core, so 4b's normalizer handles AS-issued and
  Auth0-issued machine tokens identically. Needed only for a self-hosted instance that wants
  agents.

**Bridge, if an agent must go live before Stage 4:** `authorization_code + refresh_token` as
a dedicated login works with temper-mcp as-is (one-time browser consent). It is the escape
hatch, not the destination. **Avoid** the `user`-subject-as-a-human path — it proxies as
that human and conflates authorship.

Spec:
[../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md)
(Stage 4). Auth0 M2M app provisioning is an operator/console runbook step outside the repo.
</content>
