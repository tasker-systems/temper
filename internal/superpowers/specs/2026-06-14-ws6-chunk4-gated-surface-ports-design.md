# WS6 chunk 4 — gated surface ports: design

**Date:** 2026-06-14
**Workstream:** 6 (migration / convergence), `substrate-kernel-to-cognitive-map`
**Predecessors:** chunks 1–3 landed (PRs #134, #135 spec-only gate decision, #136 synthesis + parity harness). Adjudication master: `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md` (§D deployment, §9 read homes).

## Context

Chunks 1–3 changed **zero production behavior**: chunk 1 was `temper_next`-artifact-namespace only; chunk 2's install migration is strictly additive (new tables alongside live ones, synthesis an explicitly-invoked operation); chunk 3 is read-only parity tooling. The new substrate can now be **rebuilt from live production at any moment** (`temper-next synthesize`) and **read-diffed against production** (the §9 parity-read harness over list / show+meta / body / FTS / vector / graph).

Chunk 4 is where surfaces gain the ability to **dispatch reads/writes to the new substrate** — but, per §D, every port lands **gated OFF in production** behind the in-DB backend-selection flag (decided in PR #135). That gate is what keeps chunk 4 incremental: each port PR is dead-pathed in prod and merges one-reviewed-at-a-time, instead of the first live repoint forcing all surfaces into one deploy (the surfaces share one Postgres, and dual-write is rejected as split-brain). This spec settles chunk 4's decomposition and the one open architecture decision underneath it, and specifies sub-chunk **4a** (the gate itself) for immediate build.

## The reframe: chunk 4's gated core is the api+mcp server only

Backend **selection is server-side**. Both API handlers (`temper-api/src/handlers/*`) and MCP tools (`temper-mcp/src/tools/*`) construct a `DbBackend` directly against the shared `PgPool` and dispatch one operations command per inbound call. The CLI and UI are **clients of that server** — they reach it over HTTP / the MCP transport and never select a substrate.

Therefore §D's `api → cli/mcp → ui` port order is two different kinds of work:

- **api + mcp** — the substrate-switching work. This is the gated core of chunk 4.
- **cli + ui** — client adaptations to the **§5 shared-type changes** (`ResourceRef` collapse, `ManagedMeta` genericization). §D explicitly carves these out of the flip as **compile-time atomic** (one PR updating all callers), *not* runtime-gated. They are sequenced alongside chunk 4 but tracked separately; they are not part of the gate.

## The settled architecture decision: where the new-substrate `Backend` impl lives

Two invariants pull against each other:

1. **All `temper_next.*` SQL must compile against the `temper_next` search_path.** That is exactly temper-next's per-crate `.sqlx` ritual (`crates/temper-next/.sqlx`, regenerated via `cargo make prepare-next`). temper-api's `.sqlx` is pinned to `public`. So new-substrate SQL belongs in temper-next.
2. **The `Backend`-trait wiring, the `translators.rs` machinery** (managed_meta merge, body-trio population), and all surface construction live in temper-api today, which deliberately has **no temper-next dependency**.

**Decision — Approach A: a `NextBackend` adapter in temper-api delegating to temper-next.**

- temper-next exposes typed read/write operations — extending its existing `write` / `readback` / `substrate` / `synthesis` modules, plus new §9 read-home functions (graph / search / uri) — all `temper_next.*` SQL, all under temper-next's `.sqlx` cache.
- temper-api gains a **feature-gated** `temper-next` dependency and adds `NextBackend`, which implements `temper_core::operations::Backend` by delegating each method to temper-next.
- The 4a selector chooses `DbBackend` (legacy) vs `NextBackend`.

Each invariant stays home: substrate SQL in temper-next, trait + translation + surface wiring in temper-api. The new dependency edge is **acyclic** (temper-next already depends only on temper-core + leaves; temper-api → temper-next adds no cycle). Its one real cost — temper-next's onnx/embed dependency — is feature-gated and already partly present in temper-api's build via the existing ingest-pipeline feature unification (see memory: `temper-cloud` enables `ingest-pipeline` on `temper-api`).

Rejected alternatives:

- **Backend impl inside temper-next** — drags the operations / translators machinery into temper-next and splits the Backend wiring across two crates, muddying temper-next's "deterministic, declared-only region producer" scope.
- **temper-api writes its own `temper_next.*` SQL** — breaks the `.sqlx` namespace pinning (temper-api is `public`-pinned), forcing runtime queries or a second search_path cache, and duplicates the mutation-call patterns temper-next already owns.

## Decomposition

| Sub-chunk | What | Proof gate |
|---|---|---|
| **4a** *(this spec)* | The gate: a `public` config table (default `legacy`) + trivial migration + process-start cached read + a `select_backend` seam consumed by API handlers **and** MCP tools. Legacy arm wired; `next` arm returns a clean `NotImplemented` error until 4b. | flag=legacy → existing api/mcp/e2e suites byte-identical (zero prod change); flag=next → deterministic "next backend unavailable" error (new test); migration test asserts the singleton row + `legacy` default |
| **4b** | `NextBackend` **read** methods over temper-next read ops + the §9 read homes (graph / search / uri SQL, built in temper-next under its `.sqlx` cache). Gated behind flag=next. | the **chunk-3 parity-read harness, re-pointed through the live `NextBackend`** (vs direct synthesis comparison) — the §9 floor holds: set-equality on list / FTS, ordering-invariant on vector / graph (per chunk-3's two recorded findings) |
| **4c** | `NextBackend` **write** methods over temper-next mutation functions; **grows the `Backend` trait** to bring relationship/edge writes under the selectable interface (the trait is "intentionally minimal in Phase 1" and anticipates this); adapts the translators machinery (managed_meta merge, body-trio). Gated. | write-through-`NextBackend` → parity-read **equals** write-through-legacy → read, on a rehearsal Neon branch (round-trip equivalence) |
| *(carved out)* | §5 shared-type changes (`ResourceRef` collapse, `ManagedMeta` genericization) across temper-core + all callers incl. CLI/UI | compile-time atomic, one PR — **not** the gate |
| **chunk 5** | The flip (already scoped in §D) | write-freeze → final synthesis → set flag=`next` (migration) → redeploy api+mcp+cli+ui → rename legacy schema aside; rollback = flip flag back / legacy intact / Neon branch |

## 4a detailed design (this session's build)

**Flag table.** `kb_backend_selection` in `public` — it governs **surfaces**, not substrate, so it belongs in the shared schema, not `temper_next`. Singleton row enforced by a CHECK (e.g. a fixed `id` boolean primary key, the standard single-row idiom), a `backend` column constrained `CHECK (backend IN ('legacy','next'))` defaulting `'legacy'`, and an `updated_at`. The migration lives in the shared workspace `migrations/` chain; **install = `legacy` = zero behavior change.** Setting the flag at cutover is a trivial one-row migration (chunk 5), matching §D's "set/swapped by a trivial migration."

**Read semantics.** Read **once per process at startup, cached.** This is precisely §D's model — a flip takes effect on the **next redeploy** ("one config change + one redeploy"), so reading once per process is correct, not a staleness bug. Because temperkb.io is single-tenant Vercel + Neon and prod vs rehearsal are **separate Neon branches (separate databases)**, "environment-scoped" is satisfied for free by the flag living in each database. A small **test seam** lets a test inject the flag value to exercise both arms without a redeploy (the cached read takes an override in tests).

**Selector seam.** `temper_api::backend::select_backend(...) -> Box<dyn Backend>` (the `Backend` trait is verified object-safe; see its `dyn Backend` test). It replaces the ~11 `DbBackend::new` call sites across `handlers/{edges,ingest,meta,resources}.rs` and the MCP tool sites. The **legacy** arm boxes today's `DbBackend`; the **next** arm returns a `TemperError` "next backend unavailable" variant until 4b lands a real `NextBackend`.

**Relationship/edge call sites.** The current `Backend` trait carries only the six resource commands (create / show / update / delete / list / search); relationship and edge writes are concrete `DbBackend` methods, not trait methods. In 4a those call sites **stay on legacy** but **consult the flag and refuse `next`** (so we never half-switch a process into a state where resource ops would route to a substrate that relationship ops can't reach). The trait growth that brings relationship/edge writes under dispatch lands in **4c**, where `NextBackend` write methods exist to dispatch to.

**Proof for 4a.**
- Migration test: the install yields exactly one row with `backend = 'legacy'`; a second-insert / wrong-value is rejected by the CHECKs.
- flag=legacy (default): the full api / mcp / e2e suites are byte-identical to pre-4a — the seam is a pure indirection over `DbBackend::new`.
- flag=next (injected via the test seam): `select_backend` returns the "next backend unavailable" error deterministically, on both the API handler path and an MCP tool path (one e2e-level test per surface, so the wiring — not just the function — is exercised, per the established "e2e at the production caller" discipline).

## Out of scope (chunk 4 / deferred)

- 4b/4c internals (NextBackend read/write methods, §9 read-home SQL, trait growth) — their own plans, after 4a.
- The §5 shared-type changes — compile-time-atomic, carved out of the gate; its own change.
- The flip mechanics — chunk 5.
- Crate extraction (`temper-substrate` / `temper-workflow`) — post-cutover, last.
- Surface UX quality work (search/list ergonomics) beyond the §5 contract — §D out-of-scope.

## Connections

- Adjudication master: `2026-06-12-ws6-convergence-delta-adjudication-design.md` (§D deployment + the cutover-gate decision; §9 read-home floor; §5 shared-type contract)
- Chunk 2+3 plan: `docs/superpowers/plans/2026-06-13-ws6-chunk2-3-synthesis-parity.md` (synthesis + the parity-read harness 4b re-points)
- Backend trait: `crates/temper-core/src/operations/backend.rs` (object-safe, "minimal Phase 1"); `DbBackend`: `crates/temper-api/src/backend/`
- Goal record: `substrate-kernel-to-cognitive-map` WS6 (update status line when 4a lands; also correct the stale "Branch open, no PR yet" line — chunks 2+3 are merged as PR #136 / `eca9089`)
