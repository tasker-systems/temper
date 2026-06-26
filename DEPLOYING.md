# Deploying temper

Releasing and deploying are **decoupled**. [Releasing](RELEASING.md) is a single,
target-agnostic act that produces versioned artifacts (CLI binaries + a GitHub
Release). **Deploying** is per-target: each running site is an independent Vercel
project that consumes this repo on its own schedule, with its own database and
configuration. The repo's CI does **not** deploy any site.

Design rationale:
[docs/superpowers/specs/2026-06-25-multi-target-deployment-model-design.md](docs/superpowers/specs/2026-06-25-multi-target-deployment-model-design.md).

## The deployment unit: an independent Vercel project

Temper's cloud runtime is the repo-root [`vercel.json`](vercel.json), which ships the
Rust serverless functions `/api/axum` (the Axum API) and `/api/mcp` (the MCP server),
built remotely by Vercel's Rust runtime.

A **deployment target** is one Vercel project pointed at this repo. Each target owns,
on the Vercel side, everything that distinguishes it:

- its **Vercel org/project**,
- its **`DATABASE_URL`** (its own Neon database),
- its **Auth0 tenant** and the rest of its environment,
- its **deploy trigger and cadence**.

A release knows none of this. Two (and more) targets coexist without the repo
encoding any single one's identity:

| Target | Vercel project | Database | Deploy trigger |
|---|---|---|---|
| **temperkb.io** | the canonical cloud project | its Neon (`temper-cloud`) | Vercel git auto-deploy from `main` |
| **enterprise self-hosted** | the operator's own project | the operator's Neon | the operator's own Vercel git integration / schedule |

Adding a target is a Vercel-side operation: create a project, point it at the repo (or
a fork / a pinned `v*` tag), set its env + `DATABASE_URL`, choose its production
branch or promotion model. No repo change is required.

## The invariant that keeps auto-deploy safe

> **Additive-only on `main`.** Changes that merge to `main` carry only
> backward-compatible (additive) schema migrations. New code runs against the old
> schema and old code against the new, so any target auto-deploying `main` stays
> safe regardless of the exact moment its migration is applied.

This is what lets temperkb.io (and any target) auto-deploy `main` without a CI
migration gate: the steady state is additive, and additive is backward-compatible by
construction.

A **non-additive / big-bang** schema change (a rename, a destructive collapse, a
search-path flip) is **not** an ordinary merge. It is operator-run against each
target's database via an operator-gated cutover procedure, coordinated with that
target's deploy. It is never a silent `main` auto-deploy.

## Applying schema changes per target

Production migrations are **operator-run** against each target's Neon database
(boot-time `migrate!` was removed). Each target owns the order:

**back up → migrate → verify → deploy**, against its own DB.

- **Additive migration** — `sqlx migrate run` against that target's Neon with the
  canonical `search_path`. Order relative to deploy is flexible (additive is
  backward-compatible), but back up first.
- **Big-bang / search-path flip** — an operator-gated cutover: durable backup,
  cutover, verify, then the coincident redeploy. (The executed WS6 schema collapse
  that established this pattern is in git history.)

## Per-target Vercel setup (reference)

To stand up a new target (or document an existing one):

1. **Create the Vercel project** and connect it to the repo (or fork / `v*` tag).
2. **Set environment** on the project: `DATABASE_URL` (its Neon), Auth0 vars, and any
   other secrets the functions read. (The repo's `vercel.json` is target-agnostic.)
3. **Choose the production trigger** — git auto-deploy from a production branch
   (temperkb.io uses `main`), or manual promotion if the operator wants deploys gated.
4. **Provision the schema** on its Neon DB from `migrations/` before first traffic.
5. **Verify** a live read/write through the deployed `/api/axum` against its DB.

## Rollback

Each target rolls back independently via Vercel's immutable deployments:

```bash
vercel rollback                          # previous production deployment
vercel rollback <deployment-url-or-id>   # a specific one
```

`rollback` re-points the production alias with no rebuild. If the bad deploy also
applied a schema change, roll that target's schema back per the runbook before/with
the alias rollback.
