# SCIP Code-Graph Integration — Spec

**Date:** 2026-07-12
**Status:** Design proposed, not yet approved for implementation
**Scope:** The phased delivery plan for a native, event-sourced code-intelligence graph in Temper,
sourced from SCIP and built as a **sibling projection family** on the substrate kernel — kept
architecturally distinct from the curated resource/edge/cogmap graph and joined to it only by
symbol-string citation.

> **Companion research** (the full analysis and rationale): [`docs/research/2026-07-12-scip-code-graph-integration-research.md`](../../research/2026-07-12-scip-code-graph-integration-research.md).
> This spec is the *what/when/acceptance*; the research doc is the *why*. Read the research doc first.
>
> **Goal** (Temper): *Native SCIP code-intelligence graph for Temper* (`@me/temper`,
> ref `native-scip-code-intelligence-graph-for-temper-019f56e1-…`). Five research-spike tasks
> (one per vantage) `advances` it.

---

## 1. Load-bearing premise

A SCIP index is a **mechanically-generated, commit-pinned, closed-ontology projection of source at
one commit**. Temper's resource graph is **curated, assigned-identity, open-vocabulary, ledger-as-truth**.
They are different *in kind* (research §1.6), and the design keeps them different in kind:

- **Do NOT** put code symbols in `kb_resources`, code edges in `kb_edges`, or code structure through
  the region/lens/salience producer.
- **DO** reuse the substrate *kernel* — `kb_events` ledger, append-then-project, CAS blobs,
  contexts-as-home + team-DAG authz, machine principals, replay/drop-rebuild, provenance/span-locators.

The unifying idea: **a SCIP index is itself a "view from somewhere"** — code as observed from commit
`C` by tool `T` at ingest time `t`. So ingest is an *attributed event*, the queryable tables are
*replayable projections*, and every read *resolves an explicit vantage*. This is what makes a
structurally-distinct code-graph nonetheless faithful to the event ledger and "no view from nowhere".

**The membrane invariant (non-negotiable):** the two graphs touch *only* by symbol-string citation.
Code relationships never enter `kb_edges`; the `edge_kind` enum is never widened with mechanical kinds
(`calls`/`imports`); code structure never feeds the region producer. This invariant must be written
down and, where feasible, test-guarded (Phase 3).

---

## 2. Architecture at a glance

```
external indexer (CI)            Temper substrate kernel                 code-graph projection family
─────────────────────           ──────────────────────                  ───────────────────────────
rust-analyzer scip  ─┐          kb_events (append-only truth)            kb_code_indexes     (vantage)
scip-typescript      ├─ .scip ─► code_index_ingested ──project──►        kb_code_symbols     (string id)
scip-python …        ┘  blob    (payload = metadata + CAS blob_hash)     kb_code_documents
                        │                                                 kb_code_occurrences (big table)
                        └─► CAS (opaque bytes)                            kb_code_relationships

reads:  code_definition / code_references / code_implementations / code_hover / code_blast_radius
        (exact traversal over kb_code_*, vantage-resolved, context-gated — NOT graph_traverse)

bridge: curated resource ──cites (symbol string, via span-locator provenance)──► kb_code_symbols
```

Authz seam: `kb_code_indexes.context_id → kb_contexts` ⇒ code facts inherit
`contexts_readable_by` / `context_authorable_by_profile`. Deny = zero rows.

---

## 3. Data model (target DDL — ratified in Phase 0)

Full column-level DDL is in research §3. Summary of the projection family (all additive migrations;
all carry event lineage `ingested_by_event_id` / `is_superseded`):

| Table | Grain | Identity / key | Notes |
|---|---|---|---|
| `kb_code_indexes` | one per ingested SCIP index | `UNIQUE(context_id, commit_sha, tool_name)` | **the vantage row**; `blob_hash`, counts, `is_superseded` |
| `kb_code_symbols` | one per distinct symbol string (deduped across indexes) | `symbol_string UNIQUE`; `BIGINT id` = interned surrogate (storage only) | identity is the **string**; per-observation docs live index-scoped |
| `kb_code_documents` | one per `(index, relative_path)` | `UNIQUE(index_id, relative_path)` | optional link to a `kb_resources` file (citation, not merge) |
| `kb_code_occurrences` | every symbol appearance | `BIGINT id`, FKs to index/document/symbol | **the big table**; partitioned by `index_id`; role bitset + syntax kind |
| `kb_code_relationships` | per `(index, from_symbol, to_symbol)` | boolean flags (`is_reference`/`is_implementation`/…) | closed ontology, exactly SCIP's — **not** `kb_edges` |

Identity nuance: the interned `BIGINT` is a masked reference-free surrogate (diffs with `id` masked
under the replay harness's masked-surrogate rule), *not* the assigned-identity model of `kb_resources`.
Symbols are addressed externally by string.

---

## 4. Event model (additive)

One additive migration seeds these `kb_event_types` (+ payload schemas) and their `_project_*`
functions. `_event_append` rejects unseeded names, so nothing fires until registered.

| Event type | Payload (typed struct) | Projector effect |
|---|---|---|
| `code_index_ingested` | `{context_id, commit_sha, ref_name, tool_name, tool_version, project_root, text_encoding, blob_hash, counts}` | read CAS blob → fan-out expand into `kb_code_*` in one txn |
| `code_index_superseded` | `{index_id}` | flip `is_superseded`; optionally enqueue GC |
| `code_index_pruned` | `{index_id}` | projection-only GC (detach partition); **event + CAS blob persist** |

**One event per index ingest**, not per symbol/occurrence — the heavy data is CAS-referenced by
`blob_hash` (the block-content pattern). Attribution: `emitter_entity_id` = the CI indexer machine
principal (`kb_machine_clients`); `invocation_id`/`correlation_id` thread multi-tool ingests.

**Replay invariant:** drop `kb_code_*` → replay `code_index_ingested` events (re-reading CAS blobs) →
rebuild byte-identically, asserted by the extended `replay_roundtrip` harness.

---

## 5. Phased roadmap

Each phase is an additive, independently-shippable slice (matching the repo's wave/phase convention
and the additive-only-on-`main` discipline). Each maps to a research-spike task on the goal.

### Phase 0 — Spec & schema ratification
- **Deliverable:** this spec advanced to "approved"; §3 DDL and §4 event payloads finalized; the §1
  membrane invariant written down; the §7 open questions decided (occurrence retention, vantage
  default, diff semantics).
- **Spike:** *kb_code_\* data architecture & occurrence-table sizing* + real counts from
  `rust-analyzer scip` over this monorepo.
- **Acceptance:** ratified migration DDL + event-type list reviewed; no code yet.

### Phase 1 — Decoder + ingest
- **Deliverable:** `temper-scip` decoder (prost decode + symbol-string parser + validation);
  `code_index_ingested` event + projector; CAS blob storage; idempotent upload
  (CLI `temper code index` + `POST /api/code/index`, large-blob via segmented upload).
- **Spike:** *temper-scip decoder & ingest path prototype*.
- **Acceptance:** a real `.scip` ingests idempotently; a golden round-trip test (from `scip` CLI
  `snapshot`/`test` fixtures) passes; `.sqlx` caches regenerated per the SQL-macro ritual.

### Phase 2 — Read surface
- **Deliverable:** `code_definition` / `code_references` / `code_implementations` / `code_hover` as
  SQL functions + MCP tools + CLI, vantage-resolved and context-gated, returning `ts-rs`-derived DTOs.
  Replay round-trip test extended to `kb_code_*`.
- **Spike:** *code-navigation read surface & cross-index diff* (reads half) + *code_index_ingested
  event, projector & replay invariant*.
- **Acceptance:** def/refs/impls correct on the dogfood index (validated vs `scip` snapshots);
  drop-rebuild round-trip green; unauthorized reader gets zero rows.

### Phase 3 — Bridge (citation membrane)
- **Deliverable:** symbol-string citation via annotate-only span-locator provenance (Mechanism A,
  research §7.2 — extend the remote-source URI convention to carry a symbol string); optional
  `kb_code_citations` (Mechanism B) if a first-class queryable link is wanted; cogmap region → symbol
  backing. **The membrane invariant test.**
- **Spike:** *symbol-string citation seam (resource/cogmap ↔ code graph)*.
- **Acceptance:** a `decision` resource cites an exact symbol via existing provenance with **no schema
  change**; a guard asserts no code edge lands in `kb_edges` and no code structure reaches the region
  producer.

### Phase 4 — Versioning & diff
- **Deliverable:** multi-index vantage + supersession; `code_index_pruned` retention GC (working set =
  default-branch tip + open-PR heads, logged, never silent); `code_blast_radius` cross-index diff.
- **Spike:** *code-navigation read surface & cross-index diff* (diff half).
- **Acceptance:** blast-radius diff runs between two commits with a defined line-shift-stable semantics;
  pruning detaches partitions while events + blobs persist (rehydratable by replay).

### Phase 5 — (optional) fuzzy code search
- **Deliverable:** embed docstrings/signatures as chunks; wire into `unified_search` for "find similar
  code," strictly separate from structural navigation.
- **Acceptance:** semantic "similar code" query returns sensible hits; structural reads remain exact
  (no vector path leakage).

**Dogfood throughout:** index this monorepo (`rust-analyzer scip` + `scip-typescript`) into its own
context so agents navigate Temper's own code through Temper.

---

## 6. Reuse vs. build (the seam map)

| Reuse wholesale | Reuse selectively | Build new | Do NOT reuse |
|---|---|---|---|
| `kb_events` + `_event_append` + projector pattern; CAS + payload-carries-hash; contexts + team-DAG authz; `kb_machine_clients`; replay/drop-rebuild; span-locator provenance | segmented upload (large blobs); embeddings/FTS/`unified_search` (fuzzy code search only) | `kb_code_*` tables; `temper-scip` decoder; code-nav reads; commit/index versioning + diff | `kb_edges` for code edges (never widen `edge_kind`); region/lens/salience/cogmap machinery over code; `kb_resources`/UUIDv7 identity for symbols |

---

## 7. Open questions (decide in Phase 0)

1. Occurrence-table partition granularity + retention working-set policy (research §9, §10.1).
2. Vantage default when multiple tools index one commit, or the default branch lacks a fresh index (§10.4).
3. Diff semantics stable under pure line-shift — exact-per-commit vs approximate range mapping (§10.5).
4. Eager vs lazy reference materialization (expand from CAS blob on demand) (§10.1).
5. Cross-repo `external_symbols` navigation — in-repo first, cross-repo later (§10.3).
6. First indexers/languages for dogfood — `rust-analyzer scip` + `scip-typescript` (§10.6).

---

## 8. Risks

- **Scale** — `kb_code_occurrences` is the sizing driver; partition + bounded retention from day one.
- **Membrane erosion** — the whole value proposition depends on never merging the two graphs; guard it
  with an explicit invariant + test (Phase 3), analogous to the additive-only-on-`main` guard.
- **Local-symbol scoping** — `local <id>` is unique only per `(index, document)`; mis-scoping corrupts
  find-references.
- **Staleness** — an index is valid only for its commit; navigation defaults must make the vantage
  explicit rather than implying a floating "current" graph.

---

## 9. Appendix — substrate citations

All verified against `migrations/` (canonical baseline, not the retired
`docs/event-sourced-architecture-design.md` row-shapes):

- Ledger + strict event-type registration: `migrations/20260624000001_canonical_schema.sql:465-506`;
  `…02_canonical_functions.sql:765-787`.
- Replay + invariant: `crates/temper-substrate/src/replay.rs`;
  `crates/temper-substrate/tests/replay_roundtrip.rs`.
- Curated edges (kept separate): `kb_edges` `canonical_schema.sql:628-650`; `edge_kind` enum `:95`.
- Contexts + authz: `kb_contexts` `:159-168`; `contexts_readable_by`/`context_authorable_by_profile`
  `migrations/20260712000010_context_read_predicates.sql:84-124,171-199`.
- Machine principals: `migrations/20260711000010_machine_clients.sql`.
- Span-locator / annotate-only provenance (the citation seam):
  `migrations/20260710000001_block_provenance_annotate.sql`;
  `docs/superpowers/specs/2026-07-10-issue-355-annotate-only-provenance-and-span-locators-design.md`.
- Segmented ingest (large-blob upload): `migrations/20260708000012_streaming_ingest.sql`;
  `crates/temper-mcp/src/tools/ingest.rs`.
- Embeddings / search (fuzzy only): `crates/temper-ingest/src/{embed,chunk,pipeline}.rs`;
  `migrations/20260711000050_search_vector_scope_aware.sql`.
- SCIP: `scip.proto` (github.com/sourcegraph/scip); docs `scip-code.org`.
