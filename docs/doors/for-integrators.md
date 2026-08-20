# Building against Temper

For someone writing code that talks to Temper — a service, a script, or an agent runtime that
is not the CLI.

## The API

Every endpoint and schema is generated from the router, so the reference cannot drift from
what ships. The published API reference is at
[docs.temperkb.io](https://docs.temperkb.io/), rendered from the OpenAPI spec. The API base
URL is `<origin>/api` — for the hosted instance, `https://temperkb.io/api`.

Before you call anything, read **[The Trust Boundary](../concepts/trust-boundary.md)** — it
sets out the two-gate boundary the whole API sits behind, how to discover the audience, the
error contract, and the machine-principal sharp edge.

## Authenticating as a machine

A machine principal is not a user with a long-lived token; it is its own kind of principal,
registered ahead of time.

1. **[Standing up a machine credential](../playbooks/standing-up-a-machine-credential.md)** —
   issuing and using machine credentials.
2. **[Machine tokens](../concepts/machine-tokens.md)** — the token claim shape, who mints vs.
   who validates, and what a token does and does not carry.
3. **[Token verification](../concepts/token-verification.md)** — how a token is validated, and
   the one-issuer-per-instance invariant.

## What a caller is allowed to do

- **[Authoring authorization](../concepts/authoring-authorization.md)** — the rules that
  decide whether a write is permitted: explicit capability, the three predicates, and the
  container-write cascade.
- **[Contexts and refs](../concepts/contexts-and-refs.md)** — what a context is and how to
  address one. The ref grammar, the two-grammars problem, and the four traps.

## Language SDKs

- **[Ruby, via temper-rb](../sdks/temper-rb.md)** — the generated gem and how it tracks the
  API.
