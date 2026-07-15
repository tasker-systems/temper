# Deploying temper

Releasing and deploying are **decoupled**. [Releasing](RELEASING.md) is a single,
target-agnostic act that produces versioned artifacts (CLI binaries + a GitHub
Release). **Deploying** is per-target: each running site is an independent Vercel
project that consumes this repo on its own schedule, with its own database and
configuration. The repo's CI does **not** deploy any site.

Design rationale:
[docs/superpowers/specs/2026-06-25-multi-target-deployment-model-design.md](docs/superpowers/specs/2026-06-25-multi-target-deployment-model-design.md).

## The deployment unit: an independent Vercel project

Temper's cloud runtime is the repo-root [`vercel.json`](vercel.json), which ships three
Rust serverless functions — `/api/axum` (the public Axum API), `/api/mcp` (the MCP server),
and `/api/internal` (the infrastructure-invoked embed crons + server-to-server internal
routes) — built remotely by Vercel's Rust runtime. `/api/internal` is a **separate function
purely so it can carry a different `maxDuration`** (see [Function timeouts](#function-timeouts-per-function-not-per-route)
below): Vercel's timeout is per-function, and the embed crons run ONNX work that legitimately
exceeds the public API's 60s ceiling.

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

## Server-side query-embed cold starts (issue #427)

A **search** whose caller can't precompute an embedding — MCP tools, the web UI, `temper
search --text-only` — is embedded **server-side inside the request**. On a **cold** Rust
function instance that pays a one-time ONNX model load (ORT init + writing the bundled
runtime to `/tmp` + reading the quantized model + first inference). Run inline that can
exceed the query-embed budget (`TEMPER_QUERY_EMBED_BUDGET_MS`, default 8s), at which point
the vector arm is silently dropped and the search degrades to FTS + graph with a
`"vector ranking was unavailable"` diagnostic. On a low-traffic deploy that scales to zero
between searches, *every* search can pay this — the symptom is "search never uses vectors."

Two committed, target-agnostic mitigations in `vercel.json`, so every target inherits them:

- **Function memory → CPU.** All three Rust functions set `memory: 3009` (Vercel scales CPU
  with memory), so the cold model load + first inference fit inside the budget instead of
  timing out. This is the **durable** fix for the search path and it covers **every** Rust
  function (each lambda is separate). On `api/axum.rs`/`api/mcp.rs`, `maxDuration: 60` is
  headroom above the 8s search budget so a cold request completes rather than being killed.
- **Keep-warm cron.** `GET /api/embed/warm` (every 2 min) loads and exercises the embedder so
  a serving instance's model cache is hot. Same `EMBED_DISPATCH_SECRET` bearer gate as
  `/api/embed/dispatch` (fail-closed when unset), so it activates once that secret is set (see
  [Async embedding](#async-embedding-off-request-embed-drain-issue-299) above — the same secret
  gates both). It is routed to the **`api/internal`** function, so it keeps the **embed-drain
  worker** hot (post-#299 that is where the real repeated ONNX cost lives — dispatch runs every
  minute); the public Axum/MCP search paths rely on the memory lever, their 8s budget +
  graceful FTS degrade, and live traffic. Tune the cadence, or the budget via
  `TEMPER_QUERY_EMBED_BUDGET_MS`, per target.

### Function timeouts: per-function, not per-route

Vercel's `maxDuration` is set **per function** (the `functions` map, keyed by source file),
never per route — a `routes` entry cannot carry its own timeout. So an endpoint gets a
different timeout only by being served from a different function. That is why the embed crons
live on `api/internal.rs` (`maxDuration: 300`) while the public API stays on `api/axum.rs`
(`maxDuration: 60`): raising the public ceiling to survive a long cron would let **any** public
request hang for that window. The `routes` block diverts `/api/embed/dispatch`, `/api/embed/warm`,
and `/internal/*` to `/api/internal` before the `/(.*) → /api/axum` catch-all. The heavy ONNX
surface is confined to these crons by design — `search` self-bounds at the 8s query-embed budget
and degrades to FTS+graph, and `ingest` defers embedding to the drain (#299) — so no other
endpoint needs a raised timeout.

## Rollback

Each target rolls back independently via Vercel's immutable deployments:

```bash
vercel rollback                          # previous production deployment
vercel rollback <deployment-url-or-id>   # a specific one
```

`rollback` re-points the production alias with no rebuild. If the bad deploy also
applied a schema change, roll that target's schema back per the runbook before/with
the alias rollback.
