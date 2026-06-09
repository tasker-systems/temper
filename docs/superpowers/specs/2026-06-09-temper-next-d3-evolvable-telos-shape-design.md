# temper-next deliverable 3 — evolvable seed/telos data-shape

**Status:** design approved, ready for implementation plan
**Date:** 2026-06-09
**Builds on:** deliverables 1+2 (`jct/temper-next-event-firing-charter-blocks`) — content-block/chunk
correctness + event-firing parity + charter-as-content-blocks.
**Design sources:** `docs/superpowers/specs/2026-06-07-scenario-yaml-seed-dsl-design.md` (roadmap
deliverable 3); `docs/superpowers/specs/2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md`
(questions-as-blocks, telos-charter composition); goal `substrate-kernel-to-cognitive-map` (Arc 2 seed
question-set).
**Grounding discipline:** per `implementation-grounding.md` — every step carries a CONFORM / EXTEND /
AMEND tag and a `file:line` or spec citation. Claims are grounded in quoted artifact, not narrated.

---

## 1. Problem

Deliverables 1+2 made the telos charter *real content-blocks*: block-0 statement, blocks 1..n
questions-with-context, then framing blocks — persisted via the JSONB block→chunk path. They
**deliberately deferred** the rigorous, evolvable *shape* of that data. Three gaps remain:

1. **Questions-with-context is positional, not rigorous.** `TelosDef`/`QuestionDef` flatten
   `statement \n\n context` to blocks by `seq` (`crates/temper-next/src/scenario/model.rs:72-85`).
   Trajectory-bearing semantics (reinforced / decayed / superseded-with-scar) are unmodeled at the
   shape level.
2. **Framing carries no semantics.** Framing blocks exist in the model (`framing: Vec<String>`,
   `model.rs:64`) but are indistinguishable from questions in storage.
3. **The projection leaks (code-review finding #1, carried from D2).** `cogmap_questions`
   (`schema-artifact/02_functions.sql:279-293`) projects *every* telos block with `seq >= 1 AND NOT
   is_folded` as a guiding question. Any seeded `framing` block is therefore **mis-returned as a
   question**. Latent today only because no scenario seeds framing.

### Load-bearing invariants (carried verbatim from domain-B spec)

> `telos_resource_id` is **`NOT NULL`** — a cogmap without a telos is not a cogmap, and the schema
> says so.

> blocks are addressable but **not** findable — "they cannot leak into traversal or search… `block`
> is a reference/provenance kind only, never a graph-edge target."

The second invariant is **why** block-kind cannot be a graph concern and why the question/context grain
and framing both stay *inside* the charter resource as RDBMS-related blocks.

---

## 2. The decision that organizes everything: blocks are universal; "kind" is not

The telos-charter is **just a `kb_resources` vertex**. Its content-blocks are an **RDBMS has-many**
(`kb_content_blocks.resource_id` FK, `01_schema.sql:284-297`), *not* a graph relationship — which is
exactly what makes blocks addressable, attributable, and event-sourced-mutable while staying
non-findable.

Because **every** `kb_resources` participates in content-blocks as its fundamental unit, anything that
distinguishes blocks must be expressible *generically over any resource*. A `block_kind` enum column
fails this test on two counts:

- **It is not universal.** "kind" (statement / question / framing) is meaningful for a telos-charter
  and meaningless for most resources, yet a column would impose it on all of them.
- **It pre-declares a typology that drives behavior.** Saying what a block *is* gives exactly one lens
  for how it should act; variation-within-type then has to be recovered by introspecting other
  qualities. Unless the enum models a real exhaustive limit, the typology becomes brittle or
  overloaded.

The house style already resolves this: **"what kind of thing is this" is modeled as open
property-space, not a column.** `doc_type` is a `kb_properties` row, not a column — *"demoted
doctype-as-property"* (`02_functions.sql:558-561`). Block-role-as-property is the same move one level
down.

> **DECISION (AMEND of domain-B's YAGNI deferral of `block_kind`).** Block role is a `kb_properties`
> row, never a column. The domain-B spec deferred `block_kind` because the agent "always knows which
> block it is touching" — but framing is precisely the case that deferral named to wait for: a *read*
> that must distinguish three interleavable kinds. Authorized by spec D3 ("designed, not improvised")
> and by the `doc_type`-as-property precedent.

---

## 3. Block role as a property

### 3.1 Extend the owner-table invariant (AMEND, precedented twice)

```sql
-- 01_schema.sql:399 (current)
owner_table VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_resources', 'kb_cogmaps', 'kb_edges')),
```

- **AMEND:** add `'kb_content_blocks'` to the CHECK set.
- **Authorized by:** the set was *already* widened past the domain-B spec text
  (`('kb_resources','kb_cogmaps')`) to include `'kb_edges'` — *"§4a: edges carry facets"*. Adding
  `'kb_content_blocks'` is the **second** precedented widening, not a fresh break.
- **Invariant explicitly retired:** domain-B's *"a block cannot own a `kb_properties` row."*

### 3.2 The role property

- `owner_table = 'kb_content_blocks'`, `owner_id = block_id`.
- `property_key = 'block_role'` — **its own key, never `'facet'`** (guard-rail **G-A**).
- `property_value` = a JSONB **string**: `"statement"` | `"question"` | `"framing"`. Open at the value
  level (no DB enum); conventionally these three, extensible without schema change.
- **Single-label per block for now** (chosen over weighted multi-role to avoid the
  hydration/skill/SoP overhead of curating multi-role weights today).

### 3.3 Why this is segregation-safe (guard-rail G-A, grounded)

The facet/affinity reader that feeds region projection filters on **both** predicates:

```sql
-- crates/temper-next/src/substrate.rs:76-77
SELECT owner_id, property_value, weight FROM kb_properties
 WHERE owner_table='kb_resources' AND property_key='facet' AND NOT is_folded
```

A `block_role` row (`owner_table='kb_content_blocks'`, `property_key='block_role'`) is excluded by
**either** predicate alone. So a block's role is **invisible to the lens/region math by construction**
— it only surfaces where a read explicitly asks for it.

### 3.4 The named evolution seam (no future migration)

`kb_properties.weight` already exists (`01_schema.sql`, used by `facet_set`). Weighted multi-role is
therefore **purely additive later**: assert additional `block_role` rows with weights and lift the read
to a threshold. **No schema migration** is ever needed to go from single-label to weighted-multi-role.
This is the concrete sense in which the shape is "evolvable."

### 3.5 Authoring path (EXTEND the shared persist path)

Block JSONB gains a `role` field: `{seq, role, chunks:[…]}`. `_persist_resource_blocks`
(`02_functions.sql:459-496`) stamps the `block_role` property per block, with
`asserted_by_event_id` = the genesis/created event already threaded as `p_event`. Roles are **authored
in the seed** — no backfill/migration, because temper-next's write-path tests reset the namespace and
there is no production data (no-premature-backward-compat).

### 3.6 Question/context grain (CONFORM)

Question + context stays **one** `role='question'` block, context as an interior chunk
(`model.rs:72-85` `block_proses`). The block is the trajectory unit (fold = decay, provenance accretion
= reinforce); a question and its situating context must move together. Splitting context into its own
block would need a question→context pairing that **cannot be a graph edge** (blocks are non-findable),
reintroducing the positional ambiguity we are removing.

---

## 4. Read layer: full demotion to generic resource-block reads

The `cogmap_*` read family bakes telos semantics into per-cogmap SQL when they are really **generic
per-resource operations**. The artifact confirms the generic primitive already exists:

```sql
-- 02_functions.sql:244-251 — the generic full-document projection, works for ANY resource
CREATE FUNCTION resource_body_text(p_resource uuid) RETURNS text LANGUAGE sql STABLE AS $$
    SELECT string_agg(cc.content, E'\n\n' ORDER BY b.seq, ch.chunk_index)
    FROM kb_content_blocks b
    JOIN kb_chunks ch        ON ch.block_id = b.id AND ch.is_current
    JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
    WHERE b.resource_id = p_resource AND NOT b.is_folded;
$$;
```

So:

- **`cogmap_charter` is not charter-specific** — it is `resource_body_text(resolve telos_resource_id)`
  + an access gate (`02_functions.sql:266-273`). The only cogmap-specific atom is the
  `kb_cogmaps.telos_resource_id` FK resolution.
- **`cogmap_questions` is the offender** — it *inlines* a specialized block-listing (the `seq>=1`
  hardcode, the cogmap join, the provenance reinforce-aggregation) that is a generic per-resource read.

### 4.1 The reframed layer

- **Generic primitive (new):** `resource_blocks(p_resource, p_principal_kind, p_principal_id, p_role
  DEFAULT NULL)` → `(seq, block_id, body_text, role, reinforce_count, last_reinforced_at)`.
  Access-gated via `resources_readable_by` (`02_functions.sql:155`), `role` joined from the
  `block_role` property, `reinforce_count` aggregated from `kb_block_provenance`. `p_role=NULL` → all
  blocks; `p_role='question'` → questions; `p_role='framing'` → framing. **Works for any resource.**
  Generality is **role-param only** for now (a fully general `p_key/p_value` addressing primitive is
  deferred until mutation-addressing needs it).
- **Full body:** already `resource_body_text` — nothing new.
- **Cogmap-specific atom:** a tiny `cogmap_telos(p_cogmap) → resource_id` resolver (or callers join
  `kb_cogmaps`).
- **Retire `cogmap_questions` and `cogmap_charter`.** "questions"/"framing" become
  `resource_blocks(telos, …, p_role=>'question'|'framing')`; the charter body becomes
  `resource_body_text(telos)`. **No `cogmap_framing` is ever born.**

This *strengthens* the anti-`block_kind` argument: "kind" is not universal across resources, but **"a
resource's blocks, optionally filtered by a property" is** — so the questions-vs-framing distinction
lives entirely in a generic, property-filtered read.

### 4.2 `cogmap_regulation` stays out of scope (untouched)

`cogmap_regulation` (`02_functions.sql:297`) is a **graph-edge read** (regulation = concept-resources
`express`-edged *from* the telos), not a block read, so the block-demotion does not mechanically apply.
Informationally: regulation shares the same substrate (labeled, access-scoped relation-edges-and-
resources to the telos-charter); its *differentiating purpose* is that it is the agent's own
memory-space for insights/lessons on making the charter practicable. That purpose may be specific
enough that the function survives demotion when the regulation/edge-semantics deliverable lands. **D3
leaves it untouched.**

---

## 5. Trajectory-bearing questions — formalize existing mechanics, invent nothing

The mechanics already exist in the schema; D3 documents the mapping and ensures the generic read
composes with them.

| question act | mechanism | tag | cite |
|---|---|---|---|
| **reinforce** ("kept being right") | `count(kb_block_provenance) FILTER (WHERE NOT is_corrected)` — derivable-not-denormalized; rides on `resource_blocks` | CONFORM | `02_functions.sql:281-283`; `01_schema.sql:346-358` |
| **decay** ("stopped mattering") | `block_folded` → `is_folded=true`; generic reads exclude folded blocks | CONFORM | `01_schema.sql:291`; `02_functions.sql:250` |
| **supersede-with-scar** ("was the *wrong* question") | fold the block **+** scar (`kb_block_provenance.is_corrected=true`) | partial | `01_schema.sql:353` |

The **lesson-to-regulation** half of supersede-with-scar (a regulation concept-resource whose
provenance references the folded question-block) is domain-B §3/§4 = **forward seam**, not D3.

---

## 6. Framing neighborhood — built shallow, seam left open

D3 builds framing as **in-charter `role='framing'` prose blocks**: the grounding-of-purpose and
positioning language (the "we coordinate with initiative X / focus on domain-area Y / rely on external
systems A,B,C" guidance, plus the propositions that situate triage and concept/edge mutation).

The **relational neighborhood** — framing's named referents (initiative X, systems A/B/C, domain-area
Y) very likely resolving to concept-resources in *other visible-from-here cogmaps* (public/foundational
maps) via `express`/`near` edges — is a **forward seam**. It intersects D5 (access scaffold;
"visible-from-here" is a visibility/intersection question). D3 only guarantees the shape does not
preclude it: framing is a plain role-tagged resource-block, edge-able later.

---

## 7. Build decomposition

The change threads SQL → Rust → YAML and only *proves* end-to-end, so it is **one cohesive build task**
with internal TDD steps (splitting risks non-compiling / non-proving half-states), plus one tracked
non-blocking follow-up.

### 7.1 Build task — "D3: block-role property + generic resource-block reads + framing"

| Step | Tag | Cite / Authorize |
|---|---|---|
| Add `'kb_content_blocks'` to `kb_properties.owner_table` CHECK | **AMEND** | `01_schema.sql:399` (already widened for `kb_edges`); domain-B "block cannot own a property" retired |
| Block JSONB carries `role`; `_persist_resource_blocks` stamps `block_role` property | **EXTEND** | `02_functions.sql:459-496`; spec D3 "evolvable content-blocks, designed" |
| New generic `resource_blocks(resource, principal…, p_role)`; `cogmap_telos(cogmap)` resolver | **EXTEND** | builds on `resource_body_text` `02_functions.sql:244`, `resources_readable_by` `:155` |
| **Retire** `cogmap_questions` + `cogmap_charter`; rewrite `04_scenarios.sql` callers | **AMEND** | `02_functions.sql:266-293`; `04_scenarios.sql:61-79` |
| Rust `TelosDef`/`QuestionDef`/`content.rs`/`loader.rs` carry `role` through | **EXTEND** | `model.rs:53-95`, `content.rs:28-83`, `loader.rs:64-82` |
| Scenario authors a `framing` block; `scenario.schema.json` + JsonSchema snapshot updated | **EXTEND** | `scenarios/onboarding-cogmap.yaml`, `scenarios/scenario.schema.json`, `tests/scenario_schema.rs` |
| Regenerate `.sqlx` (`cargo make prepare-next`) | **CONFORM** | CLAUDE.md temper_next `!`-macro ritual |

**Regression gate:** `scenario_roundtrip` + the cross-path membership proof, **plus a new assertion** —
a scenario seeding a framing block proves `resource_blocks(telos, p_role=>'question')` returns **0**
framing rows (the leak is closed) and `p_role=>'framing'` returns it. Run under
`--features artifact-tests` (write-path, namespace-owning) per CLAUDE.md.

### 7.2 Follow-up task (non-blocking, UI-last) — "D3 doc/UI name-sweep"

Reconcile the public cognitive-maps docs + temperkb.io UI that name the retired reads:
`docs/cognitive-maps/03-what-lives-in-a-map.md`, `04-how-a-map-grows.md`, `07d-insights.md`; UI
diagrams `LearningActsDiagram.svelte`, `ProvenanceChainDiagram.svelte`, `ResourceBlockERD.svelte`; UI
public pages `what-lives-in-a-map`, `how-a-map-grows`, `operating-temper/insights`. Reframe the named
reads as generic-read operations. Tracked separately so the data-shape build is not gated on UI.

---

## 8. Scope boundaries

**In D3:** role-as-property + owner-table AMEND; persist-path role stamping; full read-layer demotion
(retire `cogmap_charter` + `cogmap_questions` → `resource_body_text` + generic `resource_blocks` +
`cogmap_telos`); Rust model/loader/content carry role; scenario authors framing; scenario-roundtrip +
cross-path proof + framing-exclusion assertion as the gate.

**Forward seams (out of D3):** regulation-lesson half of supersede-with-scar; the relational framing
neighborhood (edges to other cogmaps' concept-resources); weighted multi-role; general `p_key/p_value`
block addressing; `cogmap_regulation` demotion (lands with regulation/edge-semantics work); the public
doc/UI name-sweep (separate non-blocking follow-up).

**Later deliverables (unchanged):** D4 (richer multi-scenario authoring + dir-driven runner + finish
the `!`-macro sweep + retire SQL scenarios); D5 (access scaffold / RBAC → Arc 1 access-wrapper); D6
(temper-next ↔ temper migration → kernel landing).
