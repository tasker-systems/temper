# Content-Block Primitive: Addressable Resource Interiority with Accreting Per-Block Provenance

**Date:** 2026-06-03
**Status:** Design — **draft**, ready for review. Grounded against the **actual built schema**
(`20260330000001_consolidated_schema.sql` + `20260401000002_chunk_content_table.sql` +
`20260404000002_resource_manifests.sql` + `20260420000005_kb_resource_revisions.sql` +
`20260420000006_chunk_dedup_functions.sql` + `20260522000001_event_ledger_unification.sql` +
`20260522100002_edges_as_projection.sql`). Supersedes the provisional DDL sketch in the
2026-06-03 conceptual-landing session note.
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion) — sibling concern to
data-model reconciliation, not a fork of it.
**Relates to:**
[`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md) (kernel slimming),
[`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) (resource = the named, edge-connected, findable unit),
[`2026-05-22-limb1-relationship-events-edge-projection-design.md`](2026-05-22-limb1-relationship-events-edge-projection-design.md) (edges-as-projection — the discipline this spec reuses for provenance).
Confidence inventory #17 (part-attribution of a projection), #9 (forward provenance).

> **Grounding note.** This spec was written against the migrations, not against the conceptual
> session note. The note's headline open item — *"re-anchor `kb_chunks` from `resource_id` to
> `block_id`"* and its claim that "the chunk re-anchoring is the part with real migration weight" —
> was checked against the built schema and **substantially revised**. The body store, the revision
> lifecycle, the search/FTS aggregation, and the dedup machinery are all resource-scoped today; the
> design below preserves the read path intact and moves only the *write/lifecycle* anchor. Where this
> spec says "built," it has been verified against migration SQL and `temper-api/src/services/`.

---

## The problem

A concept is a KB resource (a markdown document): chunked, embedded, versioned, with a
materializable current state. We can already trace a concept's *existence and mutations* through the
event ledger — a mutation event carries a `correlation_id` back to the originating external event (a
Notion change, a GitHub PR, a Slack thread). What we **cannot** do is answer *"which third-party
system, at what point in time, shaped which **part** of this concept body"* — resolve provenance to
a region of the markdown, not just to the whole resource.

The motivating use case is **conversational interrogatability**: reading a concept the cognitive map
derived, pointing at *one section*, and asking "show me the sources for *this* — their current state,
and the provenance of this understanding." That requires **part-level addressability + part-level
provenance**.

This is confidence-inventory #17 made concrete. It is *not* attribution-by-contribution *weighting*
(a framing judgment the substrate never computes). We are after **provenance** (mechanical: which
events shaped which part, in what order), not contribution *weights* (deferred, genuinely hard).

Three needs travel together, and they generalize beyond concepts to every resource:

1. **Addressability** — reference a part of a resource by stable identity.
2. **Discrete mutation** — change one part without re-writing the whole body.
3. **Attributability** — record which events shaped each part, accreting over time.

## What the built schema actually looks like at design time

- **There is no authoritative body store.** `kb_resources` has *no* `content`/`body` column
  (`content_hash` was dropped in the manifests migration). The body text lives **only** in
  `kb_chunk_content.content` (TEXT keyed by `chunk_id`). The body you read on `temper resource show`
  is **reconstructed** by concatenating current chunks ordered by `chunk_index`, re-applying heading
  markdown from `header_path`/`heading_depth` (`resource_service.rs:438-515`). Chunks *are* the body.
- **The chunk/revision lifecycle is resource-scoped.** `kb_chunks.resource_id` (FK, CASCADE) is
  load-bearing in ≥8 places: `UNIQUE(resource_id, chunk_index, version)`; `kb_resource_revisions`
  (a revision = one body-version of the *whole* resource, `body_hash` + `chunk_count`); chunks pin
  `first_revision_id`/`superseded_revision_id`; `resource_chunks_at_revision()`; the
  `(chunk_index, content_hash)` dedup in `persist/replace_resource_chunks()`; FTS aggregation
  (`rebuild_resource_search_vector` over all current chunks `WHERE resource_id`); `unified_search`
  vec hits (`GROUP BY c.resource_id`); `graph_subgraph_nodes` first-chunk; the `kb_current_chunks`
  view (`ORDER BY resource_id, chunk_index`).
- **`body_hash` lives in `kb_resource_manifests`,** computed upstream over the reconstructed body;
  `sync_diff_for_device` compares it at the resource level.
- **Edges are an event projection** (`edges-as-projection`): a genesis `relationship_asserted` event's
  id becomes the `correlation_id` every later lifecycle event for that edge shares; the projection
  builder **keys on `correlation_id`, not on `references`** (`relationship_events.rs:4-6`). Targets
  are a tagged `{kind, value}` sum (`TargetEndpoint::{Resource, Slug}`). `kb_resource_edges` carries
  `asserted_by_event_id` / `last_event_id` / `is_folded` and is rebuildable from the ledger.

These two facts — *chunks are the only body store* and *the lifecycle is resource-scoped* — set the
whole shape below.

---

## The shape: `resource ⊃ blocks ⊃ chunks`

The resource has quietly played **two roles** because every resource has so far been a flat document,
so the seam never showed:

- **Unit of identity / access / findability / traversal** — the *named thing* other resources edge
  to, what the access layer governs, what a telos-map homes, returned-with-current-projection as the
  indexable surface. **Atomic to search and the graph.**
- **Unit of content** — where text lives, where chunk/embed/mutation/provenance happen.

**Split them.** Insert **one** grouping level. The resource stays the unit of identity. The
**content block** becomes the unit of content: addressable, discretely mutable, attributable. Chunks
stay the embedding/search index beneath blocks.

```
resource  ⊃  blocks            ⊃  chunks
(named,       (identity, order,     (embedding + text-of-record,
 findable,     provenance target,    is_current, versioned)
 atomic)       NO prose column)
```

### Two decisions that make this cheap (β, not a re-anchor)

**1. Blocks carry no prose (β).** A block has **no `content` column**. Block text stays *emergent
from its chunks*, exactly the way resource text is emergent from chunks today. We do **not** introduce
a new authoritative `block.content` store (rejected option α below) — that would be a body store the
system has never had, and would *duplicate* text (block prose + overlapping chunk windows of the same
prose). "A resource is nothing but its sequenced blocks" becomes *literally true in storage*: resource
text = concat over blocks = concat over chunks, two emergent levels with one mechanism.

**2. `kb_chunks` keeps a denormalized `resource_id`.** The chunk gains `block_id` (the new
lifecycle anchor) **and retains `resource_id`** (denormalized, immutable — block→resource is fixed at
block creation and never changes). This is what keeps the read path intact: every resource-scoped
aggregate (FTS, vec resolve-up, the search index) stays **byte-for-byte unchanged**, because it
filters `chunk.resource_id` and order doesn't matter for an aggregate.

The cascade therefore splits, and only half changes:

- **Write/lifecycle half — moves to block grain** (unavoidable; this is where per-block mutation
  lives): chunk versioning / `is_current` rotation / `(chunk_index, content_hash)` dedup / the
  revision anchor. Editing one block rotates only *that block's* chunks — a write-efficiency win over
  today's "re-chunk the whole body on any edit."
- **Read half — preserved.** FTS aggregation, `unified_search` vec resolve-up, and the resource-grain
  search index need **no structural change** (denormalized `resource_id`). Only the two
  *ordering-sensitive* reads — the body projection and `graph_subgraph` first-chunk — gain a small
  `block.seq` join.

### Three cleanly separated concerns — no conflation

| Concern | Mechanism | Supersedes? |
|---|---|---|
| Block **content** version | `kb_block_revisions` + chunk `is_current` rotation | **yes** — old chunk versions go non-current |
| Block **provenance** | `kb_block_provenance`, ordered by `accretion_seq` | **no — accretes** |
| Block **presence** (in projection + index) | `is_folded` on `kb_content_blocks` | fold = out-of-current, **preserved** |

---

## DDL

> All grounded against built tables. `uuidv7()` per repo convention. New SQL must be followed by a
> `cargo sqlx prepare` cache regen (see Plan-level questions).

```sql
-- ─── NEW: the unit of content. A projection of the block's correlation-keyed event stream. ──
CREATE TABLE kb_content_blocks (
    id                UUID PRIMARY KEY,
    resource_id       UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    seq               INT  NOT NULL,                  -- flat ordering within the resource
    is_folded         BOOLEAN NOT NULL DEFAULT false, -- out of body projection + indexing, preserved
    genesis_event_id  UUID NOT NULL REFERENCES kb_events(id),  -- correlation root of the block's stream
    last_event_id     UUID NOT NULL REFERENCES kb_events(id),  -- most recent event to change the block
    created           TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (resource_id, seq)
);
CREATE INDEX idx_content_blocks_resource ON kb_content_blocks(resource_id) WHERE NOT is_folded;

-- ─── CHANGED: kb_chunks gains block_id (lifecycle anchor), keeps resource_id (denormalized). ──
ALTER TABLE kb_chunks
    ADD COLUMN block_id UUID REFERENCES kb_content_blocks(id) ON DELETE CASCADE;  -- NOT NULL after backfill
-- resource_id is RETAINED, denormalized, immutable (= block's resource_id). All resource-scoped
-- reads (FTS/vec/search-index) are unchanged. block_id drives version/is_current/dedup going forward.
CREATE INDEX idx_chunks_block ON kb_chunks(block_id);
-- Dedup uniqueness moves to block grain (was UNIQUE(resource_id, chunk_index, version)):
--   new: UNIQUE(block_id, chunk_index, version)   [swap in the same migration, post-backfill]

-- ─── NEW: content-version anchor at block grain (replaces resource-grain revisions). ──
CREATE TABLE kb_block_revisions (
    id              UUID PRIMARY KEY,
    block_id        UUID NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    audit_id        UUID REFERENCES kb_resource_audits(id) ON DELETE SET NULL,  -- write envelope
    block_body_hash TEXT NOT NULL,
    chunk_count     INT  NOT NULL,
    created         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_block_revisions_block_created ON kb_block_revisions(block_id, created DESC);
-- kb_chunks revision pins re-point at block grain:
--   first_block_revision_id / superseded_block_revision_id  (mirror today's resource-grain pins)

-- ─── NEW: per-block provenance. ACCRETES — append-only-in-spirit, never superseded. ──
CREATE TYPE provenance_source_kind AS ENUM ('event', 'resource');
CREATE TABLE kb_block_provenance (
    id                   UUID PRIMARY KEY,
    block_id             UUID NOT NULL REFERENCES kb_content_blocks(id) ON DELETE CASCADE,
    source_kind          provenance_source_kind NOT NULL,
    source_id            UUID NOT NULL,                          -- the contributing event/resource
    contributed_by_event_id UUID NOT NULL REFERENCES kb_events(id),  -- the block_mutated event that added it
    accretion_seq        INT  NOT NULL,                          -- monotonic order this source shaped the block
    is_corrected         BOOLEAN NOT NULL DEFAULT false,         -- rare: "this source was wrong" (a scar)
    created              TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (block_id, source_kind, source_id, contributed_by_event_id)
);
CREATE INDEX idx_block_provenance_block  ON kb_block_provenance(block_id) WHERE NOT is_corrected;
CREATE INDEX idx_block_provenance_source ON kb_block_provenance(source_kind, source_id);
```

The resource-level sync hash is **unchanged in role** but moves homes (A1, resolved 2026-06-04): it
becomes a hash over the ordered `(block_id, block_body_hash)` tuples of the resource's non-folded
blocks, written to **`kb_resources.body_hash`** (a denormalized column added by the data-model spec §1)
rather than `kb_resource_manifests.body_hash` — because the data-model spec dissolves
`kb_resource_manifests` and this spec retires `kb_resource_revisions`, so neither old home survives.
Sync stays a single resource-level hash compare; `sync_diff_for_device` just reads `kb_resources.body_hash`.
Otherwise `kb_resources` is structurally untouched by *this* spec (the `body_hash` column is the
data-model spec's to add).

---

## Read path

1. **Body projection** (`temper resource show`, sync hashing): `blocks WHERE NOT is_folded ORDER BY
   seq` → within each, `chunks WHERE is_current ORDER BY chunk_index` → concat, re-applying heading
   markdown. Built from blocks + chunks; the **only** read that gains a `block.seq` join. Folded
   blocks are skipped.
2. **Search / embedding** (FTS aggregate, `unified_search` vec resolve-up): **unchanged** SQL — still
   an aggregate over `chunk.resource_id` (denormalized). Resolves up to the resource, the searchable
   surface. *Addressable ≠ findable:* blocks are not in `kb_resources`, so they cannot leak into
   traversal or search — they were never rows those paths can see. No suppression guards.
3. **Current-and-indexed invariant:** a chunk participates in the current body + indexes iff
   `chunk.is_current AND NOT block.is_folded`. Folding a block removes its chunks from the current
   body, FTS, and vector search at once.

---

## Write path

A block mutation is **one act with three honest consequences**:

1. **Content** — new `kb_block_revisions` row; that block's chunks rotate (`is_current=false` on old,
   insert new at the next version). Other blocks untouched (vs. today's whole-resource re-chunk).
2. **Provenance** — the `block_mutated` event carries the ordered sources it incorporated; the
   projection **appends** them to `kb_block_provenance` at the next `accretion_seq`. Prior provenance
   is never rewritten.
3. **Head** — `kb_content_blocks.last_event_id` advances to this event.

Block-scoped analogues of `persist/replace_resource_chunks` (e.g. `persist_block_chunks(block_id,
audit_id, block_body_hash, chunks)`) carry the same trigger-gating + single search-rebuild discipline,
scoped to the block's resource for the FTS rebuild.

---

## The ledger: one block-lifecycle event family

Provenance is **not** its own event family — it is a side-aspect of the block's change-stream. The
block is the correlation-keyed projected entity, exactly like an edge:

```
block_created  →  block_mutated  →  block_mutated  →  block_folded
(genesis;          (new block-rev,    (accretes E3;      (is_folded=true; out of
 its id =           chunks rotate;      no supersession    current projection +
 correlation_id)    accretes E1,E2)     of E1,E2)          indexing; preserved)
```

New typed payloads in `temper-core/src/types/block_events.rs` (mirroring `relationship_events.rs`),
registered in `kb_event_types`:

```rust
/// Where a block's content came from. Tagged {kind, value}, like TargetEndpoint.
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ProvenanceSource { Event(Uuid), Resource(Uuid) }

/// `block_created` — genesis. Its event id becomes the block's correlation_id.
pub struct BlockCreated { pub resource_id: Uuid, pub seq: i32 }

/// `block_mutated` — content change + provenance accretion. Reasoning rides in event `metadata`.
pub struct BlockMutated {
    pub block_id: Uuid,
    pub block_body_hash: String,
    pub incorporated: Vec<Incorporation>,   // ordered: the agent's selection sequence (may be empty)
}
pub struct Incorporation { pub source: ProvenanceSource, pub seq: i32 }

/// `block_folded` — out of current projection + indexing; preserved, not wrong.
pub struct BlockFolded { #[serde(default, skip_serializing_if="Option::is_none")] pub reason: Option<String> }

/// `block_provenance_corrected` — rare: a recorded source was wrong; carries a scar.
pub struct BlockProvenanceCorrected { pub source: ProvenanceSource, pub scar: String }
```

- **Correlation discipline:** the `block_created` event id is the block's `correlation_id`; every
  later block event shares it. The projection builder groups by `correlation_id` to compute current
  block state (content head + `is_folded`) — same contract as the edge projection builder.
  (Naming note, coherence pass 2026-06-04: `kb_content_blocks.genesis_event_id` is this projection's
  spelling of the correlation-root the sibling projections call `asserted_by_event_id` — `genesis_` is
  deliberate, naming the block's *birth* event; the role is identical. Left distinct; flagged so the
  divergence reads as intentional, not drift.)
- **Addressing vocabulary:** add `kind: 'block'` to the ledger `references` target vocabulary so
  events can carry `{kind:block, value:B}` for "what touched block B" sweeps. **`block` is a
  reference/provenance kind only — never a graph-edge target.** Resource stays the named thing edges
  connect (preserves the access-model boundary).

### The payoff query

*"Show me the sources for this section, current state + provenance"*:

```sql
SELECT p.source_kind, p.source_id, p.accretion_seq
  FROM kb_block_provenance p
 WHERE p.block_id = $1 AND NOT p.is_corrected
 ORDER BY p.accretion_seq;
-- then source_id (kind='event') → kb_events.correlation_id → external system;
-- pull each source's current state via MCP.
```

Source of truth is the ledger (append-only, correlation-linked). `kb_block_provenance` is the
**rebuildable current-state projection** we query against — the same source-of-truth/projection
contract as `edges-as-projection`: rebuild from the ledger any time; correct without mutating history.

---

## Migration & bootstrap (degenerate-case first, no history loss)

At cutover **every resource is one block.** A flat document is the degenerate case (one block);
dense multi-block resources arrive only with the synthesis agents (see Scope boundary). The migration:

1. **Mint one genesis block per resource** — `kb_content_blocks(id, resource_id, seq=0)`. Synthesize
   a `block_created` genesis event per resource (emitter = `owner_profile_id`, `occurred_at` =
   resource `created`) so `genesis_event_id`/`last_event_id` are real ledger history — the same
   pattern `edges-as-projection` used to synthesize genesis `relationship_asserted` rows.
2. **Re-anchor chunks** — `kb_chunks.block_id =` that genesis block (`resource_id` already correct;
   stays). Then swap `UNIQUE(resource_id, chunk_index, version)` → `UNIQUE(block_id, …)` and set
   `block_id NOT NULL`.
3. **Hydrate `kb_block_revisions` from `kb_resource_revisions` 1:1** — each resource-revision becomes
   a block-revision of the single genesis block. Degenerate case ⇒ `block_body_hash == resource
   body_hash`; `chunk_count` and `audit_id` carry straight over. One block ⇒ the map is unambiguous.
4. **Translate chunk pins 1:1** — `first_block_revision_id` ← (block-rev hydrated from the chunk's
   `first_revision_id`); same for `superseded`. Full point-in-time history preserved.
5. **`kb_block_provenance` ships empty** — there are no synthesis agents yet to assert provenance.

**`kb_resource_revisions` is retained through the transition** as the hydration source and a
verification reference; it is retired only after `kb_block_revisions` and the reworked point-in-time
function (`block_chunks_at_revision`, with a resource-level wrapper that composes across blocks)
are proven. Do **not** drop it in the same migration that creates the block-grain revisions.

---

## Scope boundary — one implementable plan

**Ships in this spec** (zero user-visible behavior change — a one-block resource reads exactly as
today):

- `kb_content_blocks`, `block_id` on `kb_chunks` (+ retained `resource_id`), `kb_block_revisions`,
  `kb_block_provenance`, the block event family + registry rows + `block` reference-kind.
- The β lifecycle re-anchoring (write path block-scoped; read path preserved).
- The degenerate one-block bootstrap + 1:1 revision hydration.
- The provenance **mechanism** — table + event types + projection builder — testable with fixtures.

**Rides with the synthesis-agent work** (the triage architecture this model presumes):

- **Multi-block genesis heuristics** — *what makes a block a block* (agent-declared boundaries vs.
  heading-derived vs. hybrid). At cutover everything is one block, so this is deferred with no cost;
  a misjudged boundary is a cheap intra-resource re-split, not a leaked orphan in global search.
- **Provenance population** — `block_mutated` events emitting real `incorporated` sources;
  `kb_block_provenance` stays empty until then.

**YAGNI-deferred:** `block_kind` taxonomy (prose/heading/…) — not introduced v1; nested
`(parent_block_id, seq)` — flat `seq` only, painful to remove if added speculatively, cheap to add
later if a concrete need forces it.

---

## Alternatives rejected

1. **Byte-offset / heading-anchor provenance.** Pin events to byte ranges or heading text.
   *Rejected:* documents are mutable; offsets cascade on any edit, heading anchors break on
   rename/reorder. A block is a *logical identity*, not a position — rephrase/reorder/rename leave its
   UUID intact.
2. **A concept-specific `concept_section` unit.** *Rejected* twice: smuggles in subconcepts (forces a
   "section or its own concept?" classification), and is a third thing needing its own
   identity/versioning/materialization/soft-delete — reinventing resource machinery for a special case.
3. **Concept-as-DAG-of-resources (`contains` edges).** Reuses existing machinery with no new
   primitive. *Rejected against findability:* if constituents are real `kb_resources`, traversal +
   pgvector + FTS see them by default; "not independently findable" would need suppression guards on
   every search/traversal path — leak-prone, and a smell. The reused behaviors are exactly the ones
   we'd have to disable. The resource must stay the *named, edge-connected, findable* unit.
4. **Pure event-level provenance, no spatial pin.** *Rejected:* under-delivers — you can't point at
   *a part* and ask.
5. **Overlay keeping the chunk lifecycle resource-scoped** (chunk gains a nullable `block_id`, but
   versioning stays resource-grain). Tempting as the lowest lift. *Rejected:* the unit of *mutation*
   dictates the unit of *chunk-lifecycle*. A block is independently mutable, so re-chunk fires on the
   block-mutation signal; chunk versioning is naturally block-bound. The overlay can't express
   independent block mutation.
6. **α: `block.content` as authoritative text** (chunks purely derived from it). *Rejected on
   storage-mechanics:* the system has no authoritative body store — chunks are it. Introducing
   `block.content` creates a new store and *duplicates* text (block prose + overlapping chunk
   windows). β keeps the existing "text emergent from chunks" mechanism one level down.
7. **A dedicated provenance event family** (`provenance_asserted` parallel to block events).
   *Rejected:* provenance accretes as a side-aspect of each `block_mutated` act — one event, three
   consequences. A parallel family would double the ledger writes and split the block's lifecycle
   across two correlation spines.

---

## Plan-level questions (resolve during implementation planning)

1. **Block-fold → indexing mechanic.** Does folding a block materialize as chunk `is_current=false`,
   or ride as a `NOT block.is_folded` join predicate on the current views + a partial-index revision?
   (HNSW is `WHERE is_current=true`; a join predicate keeps the index simple but returns folded-block
   chunks for the join to discard — slight recall waste. Setting `is_current=false` on fold is cleaner
   for reads but overloads `is_current`'s "non-current version" meaning.) Decide with the views in hand.
   — **RESOLVED (2026-06-04, schema.sql prep):** the **join-predicate + partial-index** path, *not*
   `is_current=false`. The decisive framing: **folding is an act on visibility** — the same category as
   folding an edge — and is fully *orthogonal* to currency. `is_current` stays a true statement about
   the chunk (it remains the latest revision whether or not its block is folded); `NOT is_folded` is the
   separate availability gate. The current views carry both predicates (`is_current AND NOT is_folded`),
   and the HNSW/vector index is built **partial** on that same combination so no folded-block chunk
   reaches the join — pure semantics, no recall waste, no overload of `is_current`.
2. **`block_chunks_at_revision` + resource wrapper** — reshape `resource_chunks_at_revision` to
   block grain and add a resource-level composer; confirm the dedup replay-guard (most-recent-revision
   `body_hash` check) translates to block grain.
3. **`resource.body_hash` as a hash over ordered block hashes** — confirm the merkle composition is
   stable for sync. ✓ **Cross-spec gap A1 (RESOLVED 2026-06-04):** the resource-level sync hash now lives
   on a **denormalized `kb_resources.body_hash` column** (data-model spec §1), since the data-model spec
   dissolves `kb_resource_manifests` and this spec retires `kb_resource_revisions` — neither old home
   survives. `sync_diff_for_device` reads `kb_resources.body_hash` (a one-column source swap, no
   aggregate). The remaining plan-level confirmation is only that the block-mutation write path
   recomputes the merkle correctly. See the reciprocal note in
   [`data-model-reconciliation`](2026-06-01-data-model-reconciliation-design.md) §3/§1.
4. **`cargo sqlx prepare --workspace`** cache regen after the new SQL; watch the per-crate
   feature-gated cache caveat (see CLAUDE.md SQL section).
5. **Crate topology** — where `kb_content_blocks` + the block event family land in the
   substrate/cogmap split the data-model reconciliation spec is sequencing. **This is effectively
   already decided** by the *"does the kernel interpret the content?"* carve-out test
   ([`data-model-reconciliation`](2026-06-01-data-model-reconciliation-design.md) §0,
   [`map-regions`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) §1): block storage
   *stores + gates* but never interprets block *meaning* (the agents' job) → **substrate kernel**,
   the same answer that landed `kb_cogmap_regions` there. Citing the test rather than re-deriving it.

---

## Summary

Insert one grouping level — `resource ⊃ blocks ⊃ chunks` — where **blocks carry no prose** (text
stays emergent from chunks, β) and **chunks keep a denormalized `resource_id`** (read path intact).
Move only the *write/lifecycle* anchor to block grain. Three separated concerns: content versions
(supersede), provenance (**accretes**, never supersedes), presence (`is_folded`, preserved). The
block is a correlation-keyed projected entity over a single event family; provenance is the accreting
side-aspect of its `block_mutated` stream, materialized into `kb_block_provenance` — ledger as source
of truth, table as the query surface, exactly the `edges-as-projection` contract. Migration starts
from the degenerate one-block case and hydrates block-revisions 1:1 from the retained
`kb_resource_revisions`, with zero history loss and zero user-visible change. Multi-block genesis
heuristics and provenance population ride with the synthesis agents; the mechanism ships now.
