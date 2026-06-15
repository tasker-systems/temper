# WS6 chunk 4b — new-substrate read path: design

**Date:** 2026-06-15
**Workstream:** 6 (migration / convergence), `substrate-kernel-to-cognitive-map`
**Predecessors:** 4a landed the gate (`kb_backend_selection` flag + `select_backend` / `require_legacy_backend` seam, gated OFF) on `jct/ws6-chunk4-gate-decomposition`. Chunk 3 built `temper-next::readback` (the §9 read homes over `temper_next.*`) + `tests/parity_reads.rs`.
**Master:** `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md` (§9 read-home floor, §D deployment); chunk-4 decomposition: `2026-06-14-ws6-chunk4-gated-surface-ports-design.md`.

## Context

4a made surfaces *able to select* a substrate but left the `next` arm erroring. 4b makes **reads** answerable from the new substrate behind `flag=next`, still gated OFF in production. The §9 read SQL already exists: chunk 3's `readback` module implements `list` / `meta` / `body` / `fts_search` / `vector_search` / `neighbors` over `temper_next.*`, each parity-proven against the matching production read on a synthesized prod-shape fixture. So 4b is predominantly **wiring** — exposing `readback` through the surfaces' read paths — not new SQL.

## Scope decision: data-parity floor, access-scoping is a flip prerequisite

`readback`'s reads are **deliberately visibility-unscoped** (its own module doc: "Access over `temper_next` is WS2's concern, not this §9 floor"). Production reads scope through `resources_visible_to(profile)` (list) or a `visible` CTE (FTS/vector). WS2's producer-intersection access model is scenario-proven on the artifact but **not built into production**.

**Decision:** 4b delivers reads at the **§9 data-parity floor** — unscoped, gated OFF. **Access-scoping over `temper_next` (the `resources_visible_to` equivalent) is a named prerequisite of the flip (chunk 5), tracked to WS2**, not built here. This keeps 4b incremental and matches chunk 3's framing. Because the reads are gated OFF, the unscoped surface never serves production until both 4c (writes) and WS2 (access) land — the flip's precondition list grows by one explicit item, recorded here and in the goal record.

## Architecture (Approach A, continued)

### `NextBackend` in temper-api
A new `NextBackend` (feature-gated `temper-next` dep) implements `temper_core::operations::Backend`:

- **Read methods delegate to `readback`**, mapping its shapes to the trait's:
  - `show_resource` → `readback::meta` + `readback::body`, reconstructed into `ResourceRow`.
  - `list_resources` → `readback::list`, mapped to `Vec<ResourceSummary>`.
  - `search_resources` → `readback::fts_search` (or `vector_search` per the command's mode), mapped to `Vec<SearchHit>`.
- **Write methods stub** `Err(TemperError::NotImplemented("… (WS6 4c)"))` — `create_resource` / `update_resource` / `delete_resource`. 4c fills them.

`select_backend`'s `next` arm **constructs `NextBackend`** instead of erroring. The legacy arm is unchanged.

### Read selector for the service-direct handlers
`list` / `search` / `get_meta` / `body` / `edges`(neighbors) bypass the `Backend` trait by design (the 4a finding — reads are service-direct passthroughs; the trait's projections are lossy and don't cover `get_meta`/`body`/`edges`). 4b adds a small **read selector** mirroring `select_backend` but for these non-trait reads: a per-handler dispatch that, under `next`, calls the matching `readback::*` function and maps its output to the handler's existing response type; under `legacy`, calls the existing service unchanged. This preserves the service-direct read architecture rather than forcing every read through the lossy trait.

The selector reads `state.backend_selection` (already on `AppState` from 4a). Both API handlers and MCP read tools route through it.

### What stays erroring under `next`
Writes: `NextBackend`'s write stubs + the relationship/edge sites still on `require_legacy_backend`. The gate is deliberately **half-open** at the end of 4b — reads answer from `temper_next`, writes refuse. That is a coherent intermediate: no production traffic hits it (gated OFF), and the next sub-chunk (4c) closes writes.

## Proof gates

1. **Re-pointed parity harness.** The chunk-3 parity tests currently call `readback::*` directly. 4b re-points them (or adds a sibling layer) so they drive reads through `NextBackend` / the read selector — proving the **wiring layer** preserves the §9 floor, not just the underlying SQL. The floor is unchanged: set-equality on `list` / `fts`, ordering-invariant on `vector` / `neighbors` (per chunk 3's two findings — list/FTS ordering are not migration invariants; vector/graph are).
2. **HTTP read-set-equality.** An integration test seeds `public`, runs synthesis into `temper_next`, then asserts that under `flag=next` the read endpoints (`GET /api/resources`, show, meta, FTS/vector search, edges) return the same result **set** as under `flag=legacy` over the same fixture. This proves the surface wiring end-to-end, the analogue of 4a's gate-wiring e2e test.

## Out of scope (deferred)

- **Writes** over `temper_next` — 4c (`NextBackend` write methods, trait growth for relationship/edge dispatch).
- **Access-scoping** over `temper_next` — WS2 / a named flip prerequisite.
- **The flip** — chunk 5 (write-freeze → final synthesis → set `flag=next` → redeploy → rename legacy aside), gated additionally on 4c + WS2 access.
- **§5 shared-type changes** — compile-time-atomic, carved out of the gate.

## Connections

- §9 read homes + parity harness: `crates/temper-next/src/readback/mod.rs`, `crates/temper-next/tests/parity_reads.rs`.
- Gate seam (4a): `crates/temper-api/src/backend/selection.rs` (`select_backend` / `require_legacy_backend` / `BackendSelection`), `AppState.backend_selection`.
- Backend trait: `crates/temper-core/src/operations/backend.rs`.
- **PR boundary:** 4a + 4b + 4c are taken together as **one PR** off `jct/ws6-chunk4-gate-decomposition` (owner's call — the gate's runtime safety is the flag, not the PR granularity). No PR until 4c lands.
- Goal record: `substrate-kernel-to-cognitive-map` WS6 — add the access-scoping-is-a-flip-prerequisite note when 4b lands.
