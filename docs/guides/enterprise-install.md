# Enterprise Install — Ground Up

This is the spine for a first real enterprise install — it flattens the phase guides into
one sequence. Each detailed step links to its phase guide; this document is the order and
the joins, not the detail.

**Primary path:** Temper's native Authorization Server fronting your Okta SAML app (see
[self-hosting-saml.md](./self-hosting-saml.md)). Auth0 and Okta-OAuth are noted variants —
see [self-hosting.md](./self-hosting.md) and [self-hosting-okta.md](./self-hosting-okta.md)
if your organization uses one of those instead.

## What you end up with

| Outcome | Produced by |
|---------|-------------|
| Deployed API + MCP behind Okta-SAML SSO | [self-hosting.md](./self-hosting.md) deploy + [self-hosting-saml.md](./self-hosting-saml.md) |
| A first system admin | the SQL root step (irreducible) |
| Instance settings (name, gating, mode) | `temper admin settings` |
| An everyone-team every member auto-joins | `temper team create … --auto-join-role watcher` |
| An org-identity telos-charter cognitive map, born + bound | `temper cogmap create` → `temper cogmap reconcile` → `temper cogmap bind` |
| (optional) The web UI | [self-hosting.md#deploy-the-ui-optional](./self-hosting.md#deploy-the-ui-optional) |
| (deferred) The Eve steward | [vercel-eve.md](./vercel-eve.md) |

## Four phases

- **(A) Install the `temper` binary** — a prerequisite for every phase below; see
  [install.md](./install.md).
- **(B) Backend deploy + auth** — stand up the API + MCP surfaces on Vercel + Neon, wired to
  Okta SAML. See [self-hosting.md](./self-hosting.md) and [self-hosting-saml.md](./self-hosting-saml.md).
- **(C) Org bootstrap** — take the blank-but-stable install to a usable org: first admin,
  instance settings, everyone-team, org-identity cognitive map. See
  [org-bootstrap.md](./org-bootstrap.md).
- **(D) Agents [deferred]** — deploying an Eve agent (the team-self-cognition steward) against
  the instance. Not sequenced in this runbook; see [vercel-eve.md](./vercel-eve.md).

## Prerequisites

- **An `embed`-capable `temper` binary.** Org bootstrap's `cogmap create` / `cogmap reconcile`
  embed the charter client-side (ONNX). The default install bundles it; if you built from
  source, reinstall with `cargo install --path crates/temper-cli --locked --force` (see
  [org-bootstrap.md § Prerequisites](./org-bootstrap.md#prerequisites)).
- **`psql` and `DATABASE_URL_UNPOOLED`** for the DB-only steps — running migrations and the
  irreducible SQL root step that promotes the first system admin.
- **Okta admin access** to create the SAML app and configure the AS.
- **A Vercel project** to host the API + MCP surfaces (and, optionally, a second project for
  the web UI).
- **A Neon project** (PostgreSQL 17) for the instance database.
