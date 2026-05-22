# Limb 1 Core — Relationship Events & Edge Projection — Design

**Date:** 2026-05-22
**Context:** `temper`
**Mode:** plan → build
**Goal:** `resource-lifecycle-event-sourcing`
**Decision:** `2026-05-21-temper-reorients-to-event-primary-resources-and-the-graph-become-projections-of-an-event-ledger`
**Task:** `2026-05-22-limb-1-core-relationship-events-edge-projection-phases-1-2`
**Umbrella task:** `event-sourced-relationship-lifecycle`

---

## Framing — why this work exists

Limb 0 (PR #91) made `kb_events` the unified, append-only, registry-backed
ledger. Knowledge-graph edges, however, are still denormalized mutable
state: `kb_resource_edges` rows written `ON CONFLICT DO UPDATE`, with an
8-variant flat `edge_type` enum and an in-place `updated` timestamp. A
correction destroys the prior topology rather than deforming it.

This work brings edges under the same event-ledger-projection discipline
that limb 0 prepared: `kb_resource_edges` becomes a **rebuildable
projection of an append-only relationship-event stream**, never itself
canonical. That is the precondition for the semantic model's deformation,
folding, and scarification mechanics to be real for *relationships* — not
only for documents (research: `2026-05-19-event-sourced-knowledge-graph-relationships`;
prior art: `2026-05-19-sstorytime-semantic-spacetime-analysis`).

This spec covers **phases 1–2** — the buildable core: the relationship-event
schema and the projection builder. Phases 3–5 (temporal `graph-as-of-T`
queries, decay/fold/scar *mechanics*, perspective-scoped projection) are
named downstream sub-limbs with their own spec→plan cycles.

---

## Resolved gate decisions

The umbrella task carried four open model questions. Resolved in the
2026-05-22 brainstorm:

### Gate 1 — Topic placement: distributed by topic, unified by event-type

Every ledger event carries two orthogonal axes: `event_type_id` (drives
payload schema — what the projection builder dispatches on) and `topic_id`
(drives semantic class and differentiated deformation behavior). These do
separate jobs.

Relationship events therefore **distribute their topics** across the
framing schema's existing topic classes — assertion/re-type under
*Declaration*, decay/fold under *Deformation*, correction under *Judgment*
— so the framing schema's "differentiated deformation behavior by
topic-class" commitment is honored. A **unified `relationship_*`
event-type family** is what the projection builder filters and dispatches
on. The research doc's worry ("the builder must subscribe across three
topic subtrees") dissolves: the builder keys on `event_type`, never on
`topic`.

### Gate 2 — Cache reconciliation: transactional incremental apply + on-demand full rebuild

Because the reorientation collapses Temper to a single server-side write
path, a relationship event is appended server-side in a transaction. The
projection delta is applied to `kb_resource_edges` **within that same
transaction**. The default projection is therefore never stale relative
to the ledger — there is no staleness window and no "is a stale traversal
acceptable" question.

A **full rebuild** still exists, but as an explicit, idempotent
maintenance/verification operation (drop + replay all relationship events
in ledger order) — not a per-event reaction. It is also the validation
harness.

There is **one global projection**. Perspective-scoped projection (phase 5)
is a query-time filter over that single projection, not a separate
materialized table per perspective — until per-perspective read volume
proves otherwise.

### Gate 3 — Vertex lifecycle: out of scope; `kb_deferred_edges` retired

Vertices are `kb_resources` rows; their event-sourced lifecycle belongs to
**limb 3** (resource lifecycle event-sourcing). Limb 1 is edges-only:
relationship events reference vertices by `resource_id` and treat them as
pre-existing.

`kb_deferred_edges` — today a holding table for forward-reference edges
whose target does not yet resolve — is **dropped**. In the event-sourced
model a `relationship_asserted` event is appended unconditionally; the
projection builder simply does not project an edge whose endpoints do not
both resolve, and picks it up on the next apply/rebuild once the target
exists. The holding table is redundant denormalized state.

### Gate 4 — Edge typing: four-type structural enum + free-text label

Adopt SSTorytime's four ST-types as a **structural `edge_kind` enum**
carrying traversal algebra:

| `edge_kind` | algebra | role |
|---|---|---|
| `contains`  | transitive | composition / participation (part-of) |
| `leads_to`  | antisymmetric, causal/temporal order | dependency, sequence, derivation |
| `near`      | symmetric | proximity / similarity |
| `express`   | leaf attribute | has-property; faceting |

Direction is carried by a **`polarity`** sign (`forward` / `inverse`);
`near` is degenerate-forward (symmetric). This is required, not optional:
"A depends_on B" is asserted source=A/target=B, but the causal arrow runs
B→A — `depends_on` is *inverse* `leads_to`. Without polarity the traversal
cannot orient causal direction.

A **mandatory free-text `label`** carries the human/agent-legible relation
name. The four types force minimal structural precision; the label
clarifies intent. The current 8-variant `edge_type` enum migrates into
`label` values, each mapped to an `edge_kind`:

| current label | `edge_kind` | polarity | note |
|---|---|---|---|
| `parent_of`    | `contains`  | forward | composition |
| `tagged_with`  | `express`   | forward | tag is a property |
| `depends_on`   | `leads_to`  | inverse | dependency runs against causal arrow |
| `preceded_by`  | `leads_to`  | inverse | temporal sequence |
| `derived_from` | `leads_to`  | inverse | sources led to the derived thing |
| `relates_to`   | `near`      | forward | honest demotion of the vague catch-all |
| `references`   | `near`      | forward | citation / cross-reference |
| `extends`      | `leads_to`  | inverse | sub-decision B — treat as causal/derivation |

**Sub-decisions.** (A) Both `edge_kind` and a non-empty `label` are
mandatory at assertion time — no edge exists without committing to one of
the four algebras and naming the intent; a `near` edge with a generic
label is the smell the assertion path should reject. (B) Existing
`extends` rows migrate to `leads_to`; re-typing any wrong ones later is
itself a `relationship_retyped` event.

---

## Architecture

```
write — explicit (assert / retype / reweight / fold via API/CLI/MCP)
        or frontmatter extraction (ingest / resource update → assert / fold)
   │
   ▼
temper-api projection service ── one DB transaction ──┐
   │                                                   │
   ├─ append relationship event ──────────────► kb_events       (ledger — truth, append-only)
   │                                                   │
   └─ apply edge delta ───────────────────────► kb_resource_edges (projection — derived)
                                                        │
                                                        ▼
                                          graph_traverse / graph_neighbors (readers)
```

The ledger is truth; `kb_resource_edges` is a rebuildable projection. Both
commit atomically, so the default projection is never stale.

### Crate boundaries

Following limb 0's posture:

- The six relationship `EventType` variants extend the `temper-events`
  enum. `temper-events` stays `temper-core`-free and appends
  `serde_json::Value` payloads via its disciplined, registry-strict
  `append_event` path — **not** `insert_event_and_audit`, which is
  resource-audit-coupled.
- The *typed* payload structs live in `temper-core/types/` with `ts-rs`
  derives (shared-types-at-boundaries rule); the structural enums
  (`EdgeKind`, `Polarity`) mirror the Postgres enums there.
- The projection service lives in `temper-api/src/services/`, depends on
  both crates, and bridges typed payloads ↔ ledger `Value`.

### Non-concerns (explicit)

- **No VaultBackend involvement.** The write path is purely server-side.
  There is no frontmatter write-back, no manifest tail action, no
  `temper sync` interaction for relationships. Relationship state
  surfacing in the local vault derives naturally from the
  read-only-vault-as-point-in-time-projection work proceeding in parallel
  (`2026-05-21-cloud-only-vault-deprecation-design`). This work must not
  add a vault write path.
- **Frontmatter-as-concept retirement is out of scope.** This work
  rewires the *write side* of the frontmatter edge-extraction path to
  emit events. It does not remove the frontmatter input, change its
  managed/open-meta shape, touch meta hashing, or move to a `temper:`
  YAML inset — that retirement is owned by the cloud-only-vault track
  (see "Frontmatter edge-extraction rewire → Coordination boundary").
- **AGE** remains out of scope as a substrate; at most a future
  alternative projection engine, decided later.

---

## Phase 1 — Event schema & data model

### Relationship event-type family

Six event types, one unified family. Each gets a typed `temper-core`
payload struct and a topic-class assignment. Phase 1 *defines all six
payload shapes* (per the umbrella task's phase-1 description); Phase 2's
projection builder *acts on* **assert / retype / reweight / fold** —
`fold` is in scope because it is the edge-*retraction* mechanism (see
"Fold is retraction" below). The `decay` and `correct` *mechanics* (the
deformation geometry, the scar) remain phase 4; defining their payload
schemas now lets the ledger carry them beforehand.

| event type | topic class | payload (beyond ledger envelope) |
|---|---|---|
| `relationship_asserted`   | Declaration | `source` resource id, `target` (a `TargetRef` — resolved id *or* unresolved slug), `edge_kind`, `polarity`, `label`, `weight` |
| `relationship_retyped`    | Declaration | new `edge_kind` / `label` (lifecycle keyed by `correlation_id`) |
| `relationship_reweighted` | Declaration | new `weight` |
| `relationship_folded`     | Deformation | fold marker (edge preserved, removed from default projection) |
| `relationship_decayed`    | Deformation | decay delta/factor — *schema only in phases 1–2* |
| `relationship_corrected`  | Judgment    | scar payload — *schema only in phases 1–2* |

Naming follows the dominant snake_case registry convention
(`resource_created`, `body_updated`).

### Fold is retraction

A `relationship_folded` event means the edge is "preserved but moved off
the default sheet" (framing schema; research doc). That is *exactly* the
semantics of an edge that is **no longer current but was not wrong** — the
default projection excludes it (`is_folded = true`), yet the edge and its
history remain and stay time-travel-reachable. `relationship_corrected`,
by contrast, is reserved for edges that were genuinely *wrong* (a
hallucinated edge, a false is-about) and carries a scar. So edge removal
is `fold`, not `correct`, and no separate "retraction" event type is
needed — the six-type family is complete.

### Event linkage

- `relationship_asserted` is the **root** of a relationship's lifecycle:
  its `id` becomes the `correlation_id` for the whole lifecycle (mirrors
  the concept-event pattern). Every later lifecycle event for that edge
  carries the same `correlation_id` — that is the projection builder's
  lookup key, so the ledger `references` array is **not** required for
  the intra-lifecycle link.
- The endpoint resources are carried in the **typed payload** — the
  structured place the projection builder reads. (The current
  `EventReference` type references events only, not entities, so resource
  endpoints belong in the payload, not in `references`.)

### Topic / registry seeding

A migration seeds:

- `kb_topics` rows for the three class topics: `declaration`,
  `deformation`, `judgment` (top-level fqdns; per-relationship sub-topics
  are available later if segmentation needs them — YAGNI now, the
  `event_type` already segments).
- `kb_event_types` rows for the six `relationship_*` type names.

### Scope

Relationship events take a `scope_id`. Until per-context scopes exist,
they use the `public` scope seeded by limb 0. Scope refinement (matching
edge visibility to the source resource's context) is a small downstream
item, noted in Open Questions.

### Structural typing — Postgres enums

- New enum `edge_kind` (`express`, `contains`, `leads_to`, `near`).
- New enum `edge_polarity` (`forward`, `inverse`).
- Rust mirrors `EdgeKind` / `Polarity` in `temper-core`.

### `kb_resource_edges` becomes the projection

- **Add:** `edge_kind edge_kind NOT NULL`, `polarity edge_polarity NOT NULL`,
  `label TEXT NOT NULL`, `asserted_by_event_id UUID NOT NULL`,
  `last_event_id UUID NOT NULL`, `is_folded BOOLEAN NOT NULL DEFAULT false`
  (column exists now for phase-4 use; the default projection excludes
  folded edges).
- **Drop:** the `edge_type` enum column, `created_by_profile_id` (emitter
  is on the event), `metadata` jsonb grab-bag.
- **Keep as derived:** `weight`; `created` / `updated` (now projections of
  event `occurred_at` times).
- **Uniqueness:** replace `uq_resource_edge` with
  `(source_resource_id, target_resource_id, edge_kind, label, polarity)`;
  keep `chk_no_self_edge`.
- `kb_deferred_edges` is **dropped** (Gate 3).

### Known ripple — graph read surfaces

Changing the `kb_resource_edges` column shape ripples through every read
surface that exposes `edge_type`. The plan's recon step must enumerate and
update all of them:

- SQL functions `graph_traverse`, `graph_neighbors`, `graph_resource_edges`,
  `graph_subgraph_nodes` (all reference the `edge_type` enum/column; the
  migration `CREATE OR REPLACE`s them). The `p_edge_types` filter argument
  semantics change — it filters on `edge_kind` and/or `label`; the plan
  picks one and makes it explicit.
- `temper-core/types/graph.rs` response types; regenerated `temper-ui` TS
  types.
- Graph handlers, CLI `temper show --edges`, MCP graph tools.

---

## Phase 2 — Projection builder, write path, migration

### Projection service

In `temper-api/src/services/` (SQL lives in the service layer). Two
entry points:

- `apply_relationship_event(tx, event)` — given a just-appended
  relationship event, mutate `kb_resource_edges`: `asserted` upserts an
  edge row (resolving a slug `target` to a resource id, or leaving the
  edge unprojected if it does not resolve yet); `retyped` / `reweighted`
  update the projected row and bump `last_event_id`; `folded` sets
  `is_folded = true` so the edge drops out of the default projection.
  Runs *within* the append transaction. (`decayed` / `corrected` are
  recognized but no-op in phase 2 — their projection behavior is phase 4.)
- `rebuild_edge_projection(tx)` — truncate `kb_resource_edges`, replay
  every relationship event in ledger order, reproduce the edge set.
  Idempotent; doubles as the validation harness.

Because `relationship_asserted` may carry an unresolved slug `target`
(Gate 3 — `kb_deferred_edges` is retired), the create path re-projects
pending slug-target assertions when a new resource appears: after a
resource is created, the projection service resolves any
`relationship_asserted` events whose slug now matches and projects the
edge. This is the event-sourced replacement for `resolve_deferred_edges`
— no holding table, the assertion event itself is the durable record.

### Write path

Assert / retype / reweight / fold are *writes* and dispatch through the
backend trait per the CLAUDE.md service-layer rule:

- New `temper-core::operations` commands: `AssertRelationship`,
  `RetypeRelationship`, `ReweightRelationship`, `FoldRelationship`
  (`fold` is the explicit "retract this relationship" operation).
- Dispatched through `DbBackend`; each emits the appropriate
  `DomainEvent`(s).
- Each command, in **one transaction**: append the ledger event →
  `apply_relationship_event`.
- **Auth before writes:** the emitter must be able to modify the source
  resource (`can_modify_resource` or equivalent) before any append.

### Surface

Phase 2 delivers all three write surfaces for the assert / retype /
reweight / fold commands: the **API handler**, the **CLI** commands, and
the **MCP** tools — the "first-class edge mechanics — label/weight/type"
surfaces the decision doc calls for. The CLI and MCP surfaces are
mechanical once the operations commands exist; they are sequenced after
the API per the data+API-first, then CLI+MCP ordering, but land in the
same plan. All three are cloud-mode writes (POST to the API); no vault
path (see Non-concerns).

### Frontmatter edge-extraction rewire

The real edge-creation path is **not** `graph_build.rs` (that is a
vault-side CLI tool that scans markdown and writes `open_meta.references`
frontmatter — it never touches the DB edge table). Edges in
`kb_resource_edges` are written by `edge_service`, invoked during the
resource lifecycle:

- `ingest_service.rs` → `edge_service::extract_and_upsert_edges` on
  resource **create**
- `resource_service.rs` → `edge_service::reconcile_edges` on resource
  **update**

Both *derive* edges from resource frontmatter (`open_meta` relationship
fields + `temper-goal`). So edges today are already a projection — of
frontmatter, recomputed per resource write. This work changes the
*source of truth* to the event ledger.

For the validation criterion to hold (full rebuild reproduces every
edge), this path must emit events rather than upsert directly:

- `extract_and_upsert_edges` → emits `relationship_asserted` events
  (mapping each frontmatter relation field to `edge_kind` + `label`;
  unresolved targets become slug-`TargetRef` assertions, not
  `kb_deferred_edges` rows).
- `reconcile_edges` → emits `relationship_asserted` for newly-declared
  relations and `relationship_folded` for relations removed from
  frontmatter (fold = retraction; the edge was right, just no longer
  current). Unchanged relations emit nothing.

The frontmatter path emits only `assert` and `fold`; `retype` /
`reweight` come solely from the explicit API/CLI/MCP write path (a
frontmatter relation field fixes the `edge_kind`, and extraction always
uses `weight = 1.0`).

`kb_deferred_edges` and `resolve_deferred_edges` are retired here — the
slug-`TargetRef` assertion event plus create-path re-projection replace
them.

**Coordination boundary.** Frontmatter-as-edge-*source* is itself legacy:
once the cloud-only-vault work makes the local vault a read-only
projection, `open_meta.references` becomes a *rendered output* of edges,
not their input. The full retirement of frontmatter-as-concept (managed/
open-meta hashing, the dash-notation Obsidian-compatible shape, the move
to a `temper:`-prefixed YAML inset) is **out of scope for this work** and
owned by the cloud-only-vault track. This work only rewires the
extraction path's *write side* to emit events; it does not remove the
frontmatter input or change its shape.

The plan's recon step sweeps for any *other* direct writers of
`kb_resource_edges` (seed fixtures `scripts/seed-graph-fixtures.sql` /
`scripts/seed-dev-data.sql`, test fixtures) and routes or adjusts them.

### Migration of existing edges

The validation criterion ("drop and rebuild = identical traversal") forces
the approach. A plain `ALTER TABLE` backfill would leave pre-existing edges
with no ledger provenance — a full rebuild would lose them.

The migration therefore **synthesizes a genesis `relationship_asserted`
event for each existing `kb_resource_edges` row**: emitter =
`created_by_profile_id`, `occurred_at` = the edge's `created`, `edge_kind`
/ `polarity` from the 8→4 mapping table, `label` = the old enum name. The
projection is then rebuilt from the now-complete ledger. Pre-existing
edges become genuine ledger history — the same move limb 0 made for
event-types.

---

## Testing & validation

- **Unit** — payload struct (de)serialization round-trips; the 8→4
  `edge_type` mapping.
- **`test-db`** — `apply_relationship_event` for each of assert / retype /
  reweight / fold; auth gating on the write commands; the
  unique-constraint upsert behavior; slug-`TargetRef` assertion projecting
  no edge until the target resource exists.
- **`test-db`** — fold removes an edge from the default projection but a
  full rebuild still reproduces it (folded), proving fold is non-destructive.
- **e2e** — the headline invariant: assert a graph, snapshot
  `graph_traverse` / `graph_neighbors` output, run `rebuild_edge_projection`,
  assert byte-identical traversal. Plus migration fidelity: pre-existing
  edges survive a full rebuild. Plus the frontmatter round-trip: creating
  then updating a resource with relationship frontmatter emits the right
  assert/fold events and the projection matches.

Replay purity holds cleanly here — edge projection is purely structural,
so drop-and-rebuild is deterministic (unlike embeddings; see the decision
doc's carried tension).

---

## Out of scope — named downstream

Each gets its own spec→plan cycle:

- **Phase 3** — temporal query path: `graph-as-of-T` against event history.
- **Phase 4** — decay / scar *mechanics* and deformation *geometry*: the
  projection behavior for `relationship_decayed` and
  `relationship_corrected` (whose payload schemas Phase 1 defines), and
  the manifold-deformation semantics of fold/decay/correction. Note
  `relationship_folded`'s *projection* behavior (drop from default set)
  ships in phase 2 as the retraction mechanism — phase 4 owns the
  geometric/deformation reading of it, not the `is_folded` flag itself.
- **Phase 5** — perspective-scoped projection: the graph filtered and
  shaped by emitter-perspective, as a query-time filter over the single
  global projection.

---

## Open questions / risks

- **Scope granularity.** Phase 1 uses the single seeded `public` scope.
  Matching relationship-event scope to the source resource's context
  visibility is a small follow-up; benign until multi-scope exists.
- **`p_edge_types` filter semantics.** The graph traversal functions take
  an edge-type filter array. Post-migration this filters on `edge_kind`
  (structural) — the plan must confirm no caller depends on filtering by
  the old specific labels, or expose label filtering alongside.
- **`near` as the new dumping ground.** Mitigated by the mandatory-label
  rule (sub-decision A); the assertion path should reject a `near` edge
  with an empty or generic label. The plan makes this validation explicit.
- **Re-assertion semantics.** An exact-duplicate `relationship_asserted`
  (same source/target/kind/label/polarity) upserts and bumps
  `last_event_id` rather than erroring — confirm this is the intended
  idempotency.
- **`append_event` is pool-bound.** `temper_events::append_event` takes
  `&PgPool`, not a transaction — so the spec's "append + project in one
  transaction" needs a transaction-accepting variant (or an executor
  generic). The plan introduces this; it is a small, contained change in
  `temper-events`.
- **`append_event` event-type match.** `append_event` has an exhaustive
  `match write.event_type` enforcing `Supersedes`-reference invariants
  for the two `Concept*` types. Adding six `relationship_*` variants
  requires arms for them (no `Supersedes` requirement). Mechanical, but
  it will not compile until handled.
