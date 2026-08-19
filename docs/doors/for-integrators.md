# Building against Temper

For someone writing code that talks to Temper — a service, a script, or an agent runtime that
is not the CLI.

## The API

Every endpoint and schema is generated from the router, so the reference cannot drift from
what ships. It is published alongside these pages.

Before you call anything, read **[Temper auth & security](../auth/README.md)** — it sets out
the trust boundary the whole API sits behind.

## Authenticating as a machine

A machine principal is not a user with a long-lived token; it is its own kind of principal,
registered ahead of time.

1. **[Machine credentials](../guides/machine-credentials.md)** — issuing and using them.
2. **[The issuer / resource-server boundary](../auth/machine-token-contract.md)** — the
   machine-token contract, and what a token does and does not carry.

## What a caller is allowed to do

- **[Cognitive-map & resource authoring authorization](../auth/cognitive-map-authoring.md)** —
  the rules that decide whether a write is permitted.

## Language SDKs

- **[Ruby, via temper-rb](../guides/temper-rb.md)** — the generated gem and how it tracks the
  API.
