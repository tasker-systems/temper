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

Some migrations also have an **operator-run content step** after the schema lands.
Delivering or updating the L0 kernel cogmap's content (landmarks + telos charter)
is one such step — it is admin-gated and fail-closed, with its own
grant → reconcile → re-lock procedure. See
[docs/guides/l0-content-delivery.md](docs/guides/l0-content-delivery.md).

## Per-target Vercel setup (reference)

To stand up a new target (or document an existing one):

1. **Create the Vercel project** and connect it to the repo (or fork / `v*` tag).
2. **Set environment** on the project: `DATABASE_URL` (its Neon), Auth0 vars, and any
   other secrets the functions read. (The repo's `vercel.json` is target-agnostic.)
   To move embedding off the request path, also set the async-embed vars — see
   [Async embedding](#async-embedding-off-request-embed-drain-issue-299) below.
3. **Choose the production trigger** — git auto-deploy from a production branch
   (temperkb.io uses `main`), or manual promotion if the operator wants deploys gated.
4. **Provision the schema** on its Neon DB from `migrations/` before first traffic.
5. **Verify** a live read/write through the deployed `/api/axum` against its DB.

## Async embedding (off-request embed drain, issue #299)

By default, a resource created over **MCP** (`create_resource`) or **HTTP-raw ingest**
(a `content` body with no precomputed chunks) is embedded **synchronously inside the
request**. For large bodies that is slow and can brush the function timeout. The async
path moves embedding **off the request**: the create returns as soon as chunk text is
persisted (immediately full-text searchable), and the vector is backfilled by a queued
job drained on a schedule. The create return shape is identical either way — no polling.

This path is **opt-in per target** and inert until you enable it, so a target with no
drain configured keeps embedding inline and never strands chunks unembedded.

To enable it on a target:

1. **Set two env vars** on the Vercel project:
   - `TEMPER_ASYNC_EMBED=1` — flips server-computed embeds to defer. Leave unset (or
     `0`) to keep inline embedding. (Caller-supplied vectors from the CLI/API are always
     persisted inline regardless — there is nothing to defer.)
   - `EMBED_DISPATCH_SECRET=<secret>` — the bearer secret gating the internal drain
     endpoint `GET /api/embed/dispatch`. **Set this to the same value as the project's
     Vercel `CRON_SECRET`**, which Vercel sends as `Authorization: Bearer <CRON_SECRET>`
     on cron invocations. If unset, the endpoint is disabled (fail-closed) and deferred
     resources stay FTS-only until it is configured.
2. **The cron is already declared** in the repo's `vercel.json` (`/api/embed/dispatch`,
   every minute) — target-agnostic, so no per-target cron setup is needed. On Vercel it
   activates on the next production deploy of a project that has cron enabled (Pro plan;
   Hobby only allows daily crons — raise the schedule there or run the drain externally).
3. **Schema**: the drain reuses the existing `kb_workflow_jobs` queue (migration
   `20260707000001_workflow_jobs_resource_scope.sql`). Apply `migrations/` as usual
   before enabling — no separate step.

**Ordering matters**: set `EMBED_DISPATCH_SECRET` (+ confirm the cron runs) *before*
setting `TEMPER_ASYNC_EMBED=1`. If deferral is on but nothing drains, new large-content
resources are full-text-searchable but not vector-searchable until the drain catches up.

**Observability**: each drain pass returns `{ claimed, completed, failed, chunks_embedded }`.
A failed embed is retried by the queue's reaper and marked `dead` after max attempts;
`dead` jobs are the re-drive signal (a `reindex`/sweep follow-up). Operator guidance on
bulk vs interactive ingest lives in [docs/upload-lifecycle.md](docs/upload-lifecycle.md#choosing-an-ingest-surface-cli-vs-mcp);
the full design is [docs/superpowers/specs/2026-07-07-async-embedding-off-request-path-design.md](docs/superpowers/specs/2026-07-07-async-embedding-off-request-path-design.md).

## Rollback

Each target rolls back independently via Vercel's immutable deployments:

```bash
vercel rollback                          # previous production deployment
vercel rollback <deployment-url-or-id>   # a specific one
```

`rollback` re-points the production alias with no rebuild. If the bad deploy also
applied a schema change, roll that target's schema back per the runbook before/with
the alias rollback.
