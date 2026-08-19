# WS6 chunk 4c — NextBackend writes + Backend-trait growth: design

**Date:** 2026-06-15
**Workstream:** 6 (migration / convergence), `substrate-kernel-to-cognitive-map`
**Branch:** `jct/ws6-chunk4-gate-decomposition` (continues 4a + 4b; all 4× ship as ONE PR)
**Predecessors:** 4a (backend-selection gate) + 4b (new-substrate read path) landed on this branch.
**Masters:** chunk-4 decomposition `internal/superpowers/specs/2026-06-14-ws6-chunk4-gated-surface-ports-design.md`; adjudication `internal/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md` (§4 edges, §5 shared types, §7 key fate, §9 read floor, §D deployment).

## Context

4b made reads answerable from `temper_next` behind `flag=next` (gated OFF) at the §9 invariant floor: a feature-gated `NextBackend` whose reads delegate to `temper_next::readback`, with writes stubbed `NotImplemented` (`crates/temper-api/src/backend/next_backend.rs:116-150`). 4c fills in the write half so the selectable `Backend` interface carries the full surface — and, per the chunk-4 spec's 4c row, **grows the `Backend` trait** to bring relationship/edge writes under dispatch.

The chunk-4 spec fixed the 4c proof gate: *write-through-`NextBackend` → parity-read **equals** write-through-legacy → read (round-trip equivalence)*, and named the three pieces — write methods over temper-next mutation functions, trait growth for relationship/edge, translator adaptation. This spec settles **how**, grounded against the actual mutation vocabulary on disk.

## What already exists (grounding)

The write primitives `create` mostly assembles from:

- `SeedAction::{ResourceCreate, PropertyAssert, RelationshipAssert, RelationshipFold, BlockMutate}` fired through `events::fire` (emit+project in one tx) — `crates/temper-next/src/events.rs:85-185`.
- `content::prepare_blocks` — chunk (`temper_ingest::chunk::chunk_markdown`, sha256) + embed (`temper_ingest::embed::embed_texts`, bge-768) prose into `Vec<PreparedBlock>` — `crates/temper-next/src/content.rs:104-110`.
- `readback::resource_row` + `reconstruct_resource_row` (4b) — return the §9-floor `ResourceRow` from a `temper_next` id — `crates/temper-api/src/backend/next_backend.rs:46-78`.
- SQL mutation functions: `resource_create`, `block_mutate`, `facet_set`, `relationship_assert`, `relationship_fold` — `schema-artifact/02_functions.sql:706/896/830/768/795`.

## Confirmed gaps (the new work)

Grounded against `schema-artifact/`:

1. **`resource_delete`** — `kb_resources.is_active BOOLEAN NOT NULL DEFAULT true` exists (`schema-artifact/01_schema.sql:171`) and event type `resource_deleted` is registered (`schema-artifact/03_seed.sql:31`), but there is **no** `_project_resource_deleted` or `resource_delete` mutation. Production delete = soft-delete (`db_backend.rs:490-504` → `resource_service::delete`).
2. **`resource_update`** — `block_mutate` revises body, `facet_set` sets properties, but neither touches the `kb_resources.title`/`origin_uri` columns (`title` is a §9 invariant). Event type `resource_updated` is registered but unbacked.
3. **`relationship_retype` / `relationship_reweight`** — do not exist. temper_next keys edges by `kb_edges.id` (UUID PK, `schema-artifact/01_schema.sql:424`) with `last_event_id` + the event ledger threading history; only assert+fold exist so far.
4. **`relationship_assert` has no uniqueness guard** — re-asserting the same active `(source_id, target_id, edge_kind, label)` would silently create a **duplicate active edge**, which the affinity math double-counts. Production prevents this by diverting a re-assert to a reweight (`db_backend.rs:98-190`).
5. **Resource home-reanchor (context move)** — `doc_type` is a *property* (`readback/mod.rs:119`; stamped via `facet_set`, `02_functions.sql:696`), so a doctype move is already a `facet_set`. A *context* move re-points the resource's `kb_resource_homes` row (`schema-artifact/01_schema.sql:222`) and has no mutation function yet.

## Settled decisions

- **Full 4c, sequenced internally** — all of create/update/delete + the four-method relationship trait growth, committed per slice, one PR when complete.
- **Natural-key identity resolution (no schema change)** — a live write resolves caller identity by the same keys synthesis writes by: profile by `handle` (= production `kb_profiles.slug`), emitter entity `pete@<surface>` (from `cmd.origin: Surface`) for that profile, home context by `(owner, slugify(cmd.context))`. No persisted old→new id-map table.
- **Commit to the edge_id-keyed model; do NOT port `correlation_id`-as-edge-identity** — temper_next's `kb_edges.id` is the stable identity and the event ledger (keyed on `edge_id` in payloads) is the history; production's correlation-chain addressing is an indirection we shed. retype/reweight become edge_id-keyed event-sourced `UPDATE`s mirroring `relationship_fold` (`02_functions.sql:781-810`). `kb_events.correlation_id` stays for its actual purpose — *grouping a multi-event act* — never edge identity.
- **Keep the no-duplicate-active-edge *invariant*, drop the divert *mechanism*** — the partial unique index already exists (`uq_kb_edges_assertion`, `01_schema.sql:441`); make `relationship_assert` **idempotent** against it (re-assert of an active edge upserts the weight on the existing edge and returns its id) instead of hard-erroring. This preserves what production's reweight-divert protected, as a DB constraint rather than correlation-chain logic.
- **Edge handle is backend-opaque** — the `Uuid` an assert returns (correlation_id for legacy, edge_id for next) is fed back into retype/reweight/fold *within the same backend*; the gate switches the whole backend, so no runtime cross-backend id translation is needed. The trait commands' `correlation_id` field is legacy-flavored but value-opaque; **renaming it is a §5 concern** (shared-type genericization, compile-time-atomic, touches all callers).
- **`resource_update` for `title`/`origin_uri`** (approved) — title is mutable at the §9 floor, written via an event-sourced `resource_updated`.
- **`move` stays in scope, not deferred** — re-homing/re-typing is a real reorganize op. Doctype-move = the existing `facet_set` on the `doc_type` property; context-move = a new event-sourced home-reanchor function. No `NotImplemented`, no silent-ignore.
- **Proof = e2e round-trip** (approved) — not a temper-next artifact test (which can't drive the legacy `ingest_service` path).

## Design

### Component 1 — substrate mutation functions (artifact `01`/`02` SQL)

Four event-sourced functions, each following the established `_project_*` + mutation-fn + `events::fire` arm shape (model on `relationship_fold` at `02_functions.sql:781-810`):

| Function | Event type | Effect |
|---|---|---|
| `resource_delete` | `resource_deleted` *(registered)* | `is_active = false` on the target resource |
| `resource_update` | `resource_updated` *(registered)* | revise mutable `kb_resources` columns (`title`, `origin_uri`) |
| `resource_rehome` | `resource_rehomed` *(new)* | re-point the resource's `kb_resource_homes` row to a new context (the context-move half) |
| `relationship_retype` | `relationship_retyped` *(new)* | set `edge_kind`/`polarity` on an edge by `kb_edges.id` |
| `relationship_reweight` | `relationship_reweighted` *(new)* | set `weight` on an edge by `kb_edges.id` |

All edge mutations key by `kb_edges.id` and mirror `relationship_fold`'s shape (`02_functions.sql:781-810`): read the edge's home for the event envelope, append the event, project the column change + `last_event_id`. Each function: add the event-type name to the bootseed/system registry (`crates/temper-next/src/scenario/bootseed.rs` — `system_event_type_names`, the same list synthesis idempotently installs at `bootstrap.rs:105`), a `SeedAction` variant + `events::fire` arm + payload type (`payloads.rs`), regenerate the per-crate cache with **`cargo make prepare-next`**, and cover replay parity under `--features artifact-tests`.

**Plus** the edge-uniqueness invariant. *(Grounding correction, 2026-06-16: the partial unique index already exists — `uq_kb_edges_assertion` on `(source_table, source_id, target_table, target_id, edge_kind, COALESCE(label,''), home_anchor_table, home_anchor_id) WHERE NOT is_folded`, `01_schema.sql:441`. Today a re-assert hard-errors on it.)* So no new index — only make `_project_relationship_asserted` **idempotent** against it: `ON CONFLICT (… that index's columns + predicate …) DO UPDATE SET weight = EXCLUDED.weight, last_event_id = EXCLUDED.last_event_id RETURNING id` (returns the existing edge id; `asserted_by_event_id` stays on the original). Covered by assert-then-reassert ⇒ one edge / updated weight, and fold-then-reassert ⇒ fresh active edge.

**Invariant carried verbatim (adjudication §0/§3):** *"all writes through atomic SQL mutation functions that emit + project in one transaction"* and *"replay is the same code path as normal operation"* — the new functions use the `_event_append` / `_project_*` split, never a direct projection write.

### Component 2 — temper-next typed write ops

Typed Rust wrappers NextBackend calls, all `temper_next.*` under `SET LOCAL search_path TO temper_next, public`, one tx each, fired through `events::fire` (mirrors `synthesis::run`'s discipline at `synthesis/mod.rs:91-94`). Home: extend `write.rs` or a focused `writes` module.

- `create_resource(...)` → `content::prepare_blocks(body)` → fire `ResourceCreate` → fire `PropertyAssert` per `Property`-fated managed key (`synthesis::key_fate`) + every open key. Returns the new resource id.
- `update_resource(...)` → `block_mutate` (body, if present) + `facet_set` (stage/mode/effort/seq + meta keys present, **incl. `doc_type` for a type-move**) + `resource_update` (title/origin_uri if changed) + `resource_rehome` (context-move, if `move_to.context_to` present). Partial — only the fields the command carries.
- `delete_resource(id, emitter)` → `resource_delete`.
- `assert_relationship` (idempotent on the active-edge unique key) / `retype_relationship` / `reweight_relationship` / `fold_relationship`.

### Component 3 — identity resolution (NextBackend `resolve` helper)

Natural-key lookups, runtime `sqlx::query` with explicit `temper_next.`/`public.` qualification (the `synthesis::source` precedent — `source.rs:1-6` — avoids the offline-cache namespace conflict):

- **profile** → `public.kb_profiles.slug` for the caller id, then `temper_next.kb_profiles` by `handle`.
- **emitter** → `temper_next.kb_entities` by `(profile_id, name = "pete@" + surface)`.
- **home context** → `temper_next.kb_contexts` by `(owner_table, owner_id, slug = slugify(name))`.

A missing resolution is a hard error (escalate, never fabricate) — it means the substrate wasn't synthesized for that caller/context.

### Component 4 — Backend trait growth (temper-core + both impls)

Add to `temper_core::operations::Backend` (`crates/temper-core/src/operations/backend.rs:44-72`):

```
async fn assert_relationship(&self, cmd: AssertRelationship)   -> Result<CommandOutput<Uuid>, TemperError>;
async fn retype_relationship(&self, cmd: RetypeRelationship)   -> Result<CommandOutput<Uuid>, TemperError>;
async fn reweight_relationship(&self, cmd: ReweightRelationship)-> Result<CommandOutput<Uuid>, TemperError>;
async fn fold_relationship(&self, cmd: FoldRelationship)       -> Result<CommandOutput<Uuid>, TemperError>;
```

- **DbBackend** — the four methods already exist as concrete (`db_backend.rs:98-415`); move them into the `impl Backend` block (signature-identical, zero behavior change). The object-safety test already guards `dyn Backend`.
- **NextBackend** — implement via Component-2 ops.
- **Surface repoint** — the 4a `require_legacy_backend` relationship sites switch to `select_backend` dispatch (they refused `next` in 4a precisely because no `NextBackend` write existed; now it does).

### Component 5 — proof (round-trip equivalence)

Extend the 4b e2e precedent (`backend_read_path_next.rs`) under `test-db,next-backend`. For create / update / delete / each edge op: run the logical op through **legacy** (flag=legacy, `public`) and through **next** (flag=next, `temper_next`), read back, assert §9-floor equality:

- resource: the invariant subset (origin_uri / title / is_active / context_name / doc_type_name / stage / mode / effort / seq) + body-text parity (reconstructed body, not the manifest-vs-merkle `body_hash`).
- edge: state (kind / polarity / label / weight / is_folded), ordering-invariant (the §9 graph-read finding; ids are non-invariant).

Plus per-function replay parity under `artifact-tests`.

**Production-fidelity rehearsal (optional, when warranted):** the Neon MCP tool can export production into a local branch so synthesis + these writes run against the real corpus, not just the synthesized fixture — the concrete form of the chunk-4 gate's "rehearsal Neon branch." Not a per-PR gate; a tool to reach for if a write path's behavior depends on real-data shape.

## Sequencing (commit per slice)

1. Substrate work: the edge-uniqueness index + idempotent `relationship_assert`, then `resource_delete` → `resource_update` → `resource_rehome` → `relationship_retype` → `relationship_reweight` (+ event types, payloads, `SeedAction` arms, `prepare-next`, artifact-tests incl. assert-then-reassert ⇒ one edge).
2. temper-next typed write ops (Component 2).
3. NextBackend create → update → delete + identity resolution (Component 3) + round-trip e2e per op.
4. Trait growth (Component 4): move DbBackend methods, NextBackend edge methods, repoint relationship sites + edge round-trip e2e.
5. `cargo make check` + full e2e (`test-db,next-backend`) + temper-next (`artifact-tests`,`next-backend`); update goal status.

**Gotcha (carried):** any temper-api build with `next-backend` MUST set `SQLX_OFFLINE=true` (temper-next macros target the `temper_next` namespace). `git checkout HEAD -- <path>` to restore from HEAD, not the index.

## Out of scope (named, not dropped)

- **`by_uri`** + MCP `get_resource`/`list_resources` enrichment reads — deferred from 4b (slug §7-dissolved; relationship enrichment over public ids).
- **§5 shared-type changes** (`ResourceRef` collapse, `ManagedMeta` genericization) — compile-time-atomic, carved out of the gate; its own change, sequenced after 4c.
- **The flip** (chunk 5) and **access-scoping over `temper_next`** (WS2 flip prerequisite).

## Connections

- Chunk-4 decomposition: `2026-06-14-ws6-chunk4-gated-surface-ports-design.md` (the 4c row + proof gate)
- 4b read path: `2026-06-15-ws6-chunk4b-new-substrate-read-path-design.md` (the §9 floor + `reconstruct_resource_row`)
- Adjudication master: `2026-06-12-ws6-convergence-delta-adjudication-design.md` (§4/§5/§7/§9/§D)
- Goal record: `substrate-kernel-to-cognitive-map` WS6 (update status when 4c lands)
