# Atlas Beat E — the context view (and the retirement of the legacy Cytoscape graph)

**Status:** design spec, approved 2026-07-09. Implements task `019f420d`, the final beat of the
Atlas reshape (north star `2026-07-06-atlas-reshape-projection-class-north-star.md`, beats A–E).

**Origin:** the build lens on `/graph/@me` drills a context circle straight back into the vault
*list* view (`TierHome.svelte:149` hard-codes `goto('/vault/<owner>/<slug>')`, bypassing the
`vault-url.ts` spine PR #312 established), while the left-nav "Graph" link
(`ContextNavGroup.svelte:41`) lands on the legacy Cytoscape page. Two entry points, two wrong
destinations. Beat E makes them one right one.

---

## 1. The act this surface serves

The north star's surface→projection-class table already names it:

| Atlas surface | Primary act | Contract consequence |
|---|---|---|
| Cogmap panorama | Orientation (of a knowledge domain) | gate labels, encode magnitude |
| **Context view** | **Composition (builder work)** | **depth; a *workable assembly*** |

Composition's attention-contract is `(substrate, perspective, scope) → workable assembly`. "Workable"
is the operative word and it is what kills the naive design: a context view is not "draw the
context." `@me/temper` holds **945 active resources and 899 edges**. A single force graph of that is
not an assembly, it is a hairball.

So the context view is **tiered**, exactly as the cogmap door is: a Tier-0 survey of the context's
*containers*, drilling into Tier-1 force-graph compositions.

---

## 2. What the data actually says

Measured read-only against prod (`crimson-fog-23541670`), context
`00000000-0000-0000-0003-000000000001`, on 2026-07-09. Every number below drove a decision.

| fact | value |
|---|---|
| active resources | 945 |
| edges touching the context | 899 (517 internal · **382 cross-home**) |
| resources with **zero** edges | 462 (49%) |
| doc-type spread | session 407 · task 351 · research 84 · concept 66 · goal 28 · decision 9 |

Edge population, by label:

| label | kind | n | internal | cross-home |
|---|---|---|---|---|
| `relates_to` | near | 451 | 282 | 169 |
| `derived_from` | express | 207 | 0 | **207 (all)** |
| `parent_of` | contains | 191 | 191 | 0 |
| `advances` | leads_to | 16 | 12 | 4 |

Three findings that reshaped the design:

1. **`goal --parent_of--> task` (189 edges) is the containment spine**, not `advances` (16). (This
   asymmetry was itself a bug — historical tasks carried `parent_of` while `--goal` filtered
   `advances`. Filed as `019f468b`, fixed by a sibling session in `586717c2` /
   `migrations/20260709000005_backfill_goal_parent_of_to_advances.sql`.)

   > **The counts above are pre-backfill** (verified against prod 2026-07-09: 369 live `parent_of`
   > goal↔task, 11 `advances` — the migration is merged but not yet applied). The backfill *folds*
   > each `parent_of` goal→task edge and asserts `advances` **task→goal** in its place: reversed
   > direction, different `edge_kind`. Container member counts are **invariant** under that
   > conversion — but only because the container walk is undirected
   > (`source_id = node OR target_id = node`) and filters on **no label and no direction**. That is
   > therefore a load-bearing invariant of `graph_context_containers`, not an implementation detail:
   > a walk that special-cased `parent_of` would silently empty every territory the day the
   > migration lands, and one that special-cased `advances` would have shown 28 empty territories
   > the day before it. Enforce with a test that asserts identical member counts across both
   > representations.
   >
   > The backfill homes the new edge on the **task's** anchor, preferring `kb_cogmaps` over
   > `kb_contexts`, so a cogmap-homed task would move its membership edge behind
   > `anchor_readable_by_profile(cogmap)`. Verified benign today — all 369 home on `kb_contexts` —
   > but the context view must not assume it.
2. **`derived_from` is 100% cross-home** — the cogmap's distillations pointing at context
   resources. This *is* the circle↔square intersection the view must draw.
3. **Aggregators are not interchangeable.** Treating `goal|concept|decision` as containers (the
   legacy `is_aggregator` definition) yields **33 of 103 empty territories** — 8 of 9 decisions, 17
   of 66 concepts — reviving precisely the inert-circle defect north-star Decision 3 exists to kill.

| container candidate | count | avg members | max | empty |
|---|---|---|---|---|
| goal | 28 | 10.9 | 108 | 8 |
| concept | 66 | 2.9 | 9 | 17 |
| decision | 9 | 0.1 | 1 | 8 |

**Goals are the only real container.** Concepts and decisions are *content*, and belong in drills as
nodes, not on the panorama as empty rings.

Seeding Tier 0 from the 28 goals at depth 2 covers **301** resources; the remaining **644** reach no
container and are residual. Grouped by `doc_type` those are
`session 395 · task 150 · research 69 · concept 21 · decision 9` — five bounded buckets, worst drill
395, rather than one 644-node bucket.

> **Caveat, load-bearing.** This dataset evolved with the product and is under-edged and
> under-faceted; it is *not* the expected steady state. The design must degrade gracefully on it
> without being shaped by it. Every mechanism below is therefore derived from data at runtime rather
> than enumerated at design time, and each is checked in both directions: sparse (today) and dense
> (healthy).

---

## 3. Decisions

### D1 — The door: `/graph/[owner]?context=<slug>`

Mirrors the existing `?cogmap=<id>` research door. Both Home lenses then land *inside* the Atlas
shell (crumb, camera, rail, legend, filters) instead of leaving it.

```
Home (/graph/@me)
  research lens ──► ?cogmap=<id>    → cogmap panorama      (existing)
  build    lens ──► ?context=<slug> → context panorama     (NEW)

left-nav "Graph" ──► /graph/@me?context=temper   ┐ same URL,
build circle     ──► /graph/@me?context=temper   ┘ same view

/vault/@me/temper        → list view (unchanged; the interstitial's WS3 owns it)
/vault/@me/temper/graph  → redirect → the door, then deleted
```

`contextGraphHref` in `vault-url.ts` is **repointed, not removed** — it stays the single authority.
`TierHome.svelte:149` and `HomeA11yList.svelte:92` stop hand-rolling paths and route through it.

### D2 — Tier 0 = goal containers + a residual tray

The panorama draws **container territories** (goal-rooted, edge-derived, `member_count` at depth 2).
Empty containers ghost-render (`isEmptyTerritory`), they do not disappear.

Everything that reaches no container is **residual**, and residuals live in a **tray outside the
force field** — always labeled, count-badged, enterable, never competing for salience.

**Why a tray and not the field.** With residuals in the field, on real data:

- 4 of the 10 gated labels become `Unfiled · …`; the largest, brightest landmark in the map of your
  work is `Unfiled · session` (395).
- Worse, `intensityOf` normalizes against the single largest weight in the field, so one 395-item
  bucket **drags every real goal's field intensity toward zero** — `Maintenance` falls from 1.00 to
  0.16. Residuals don't merely steal labels; they flatten the whole survey.

Containers and residuals are different classes: a goal is a landmark *of the work*, a residual bucket
is a doorway *to what is not filed yet*. Two classes, two treatments.

**Degradation, both directions.** On today's under-edged vault the tray is a visible, honest
doorway to 644 unfiled resources. On well-edged data containers absorb their members and **the tray
shrinks toward nothing and vanishes**. Neither is a special case. The field never degrades as
unfiled work accumulates.

### D3 — Residuals are derived from a group-by key, never enumerated

The panorama takes a **group key** (default `doc_type`, which is just a row in `kb_properties`) and
returns counts for whatever values exist. Today: `session · task · research · concept · decision`.
Tomorrow: `stage`, a facet, a keyword — no schema change, no assumption baked in.

This is what makes "sessions" a *bucket the data produced* rather than a rule the designer invented.
Sessions are hideable through the existing `filters.docTypes` mechanism; there is no
`WHERE doc_type <> 'session'` anywhere in the new read path. (The legacy view hard-coded exactly
that: `graph_subgraph_nodes` carries `WHERE doc.dt <> 'session'`.)

A **counts-only payload** gives Tier 0 its whole shape without fetching a single node.

### D4 — Container doc-types are a parameter, not a constant

Tier 0 seeds from `p_container_types text[]`, defaulting to `ARRAY['goal']`. The evidence says goals
today; the parameter says the model does not assume that forever.

### D5 — Tier 1 = the Beat-D drill, radially inverted, shapes unchanged

The drill reuses `TierNeighborhood` → `forceNeighborhood` → `NodeChip`/`Edge` as-is.

**The visual language is invariant and must not flip between views:**
`nodeMarkShape(home) = home === 'cogmap' ? 'circle' : 'square'` (`marks.ts:11-13`). Circles are
cogmap nodes; rounded-squares (`rx = 0.32·r`) are context resources. Everywhere. Always.

**The composition inverts.** `forceNeighborhood.ts:90-93` currently keys `forceRadial` on `home`,
pushing context nodes to `rOuter` and cogmap facets to `rInner`. Correct for Beat D, where a *region*
is the subject and context-resources are the sources it distilled from. Backwards for a context view,
where the resources *are* the work and cogmap nodes are what got distilled out of them.

The fix is not to flip a boolean but to stop keying the radial on `home` at all, and key it on **which
home is the subject of this view**:

```ts
forceNeighborhood(subgraph, seeds, { width, height, coreHome })
//   region drill  → coreHome: 'cogmap'   (ideas core, sources ring)   — today's behaviour
//   context drill → coreHome: 'context'  (work core, distillations ring)
```

Shape stays keyed on `home`. Radius keys on `coreHome`. One parameter, same metaphor, no visual
vocabulary is redefined. `forceNeighborhood.radial.test.ts` gains the mirrored assertion.

### D6 — Temperature encodes the axis, not container-ness *(shipped)*

`palette.ts` already said it: *warm = authored/knowledge (regions, cogmaps), cool = workflow/context.*
A goal territory borrows the region's **form and behaviour** (sized, force-separated, label-gated,
enterable) but keeps the **cool context tint**. A warm field means "you are in knowledge"; a cool
field means "you are in your work" — no legend needed. The Tier-1 drill then reads as cool squares
with warm circles intruding, which is exactly what `derived_from` is.

Shipped in `4e0e83d6`: `TERRITORY_TINTS.context` `#6fa8c7 → #7dbae8` (hue 201→206°, sat 44→70%,
lum 61→70%, canvas contrast 6.43:1 → 7.98:1), and the axis rule written into the doc comment.

### D7 — `TierPanorama` is kind-agnostic *(shipped)*

Two defects the prototype surfaced, both independent of Beat E and fixed ahead of it (`ad324b09`):

1. The field pipeline was gated on `kind === 'region'` — labels, glow, intensity. Every
   context/cogmap territory rendered unlabelled and flat. `TierPanorama` had only ever worked for
   cogmap regions despite `TerritoryKind` admitting three variants. Weight is now
   `salience ?? member_count`.
2. `intensityOf` applies an **expansive** `^1.4` to `weight/max`. Regions survive it because
   `salience` is normalized 0..1; raw `member_count` is heavy-tailed (one goal at 108, median ~3), so
   ordinary goals pinned to the opacity floor and the field read as dead grey. Counts now take a
   `log1p` ramp first: a 3-member goal lifts `0.007 → 0.181`, a 16-member `0.069 → 0.494`. `0` still
   maps to `0`, so empty containers keep ghost-rendering; regions skip the branch entirely.

### D8 — Parity with the legacy view: port `stage`, drop `session_count`

`graph_subgraph_nodes` returns `session_count` and `stage_raw`; `AtlasNode` carries neither.

- **`stage` is ported.** Task stage is load-bearing on a builder surface. This extends
  `graph_atlas_nodes_visible`'s `RETURNS TABLE`, which requires its own `DROP` + `CREATE` migration
  (a shipped SQL function's signature cannot be widened in place).
- **`session_count` is dropped.** The `⌊N⌋` glyph existed because sessions were invisible in the
  legacy view. They are no longer: sessions are a residual bucket with a real way in.

### D9 — Retirement in two phases

`aggregator_subgraph` has **exactly one non-test caller** (the `get_subgraph` handler) — no MCP, no
CLI. Cytoscape has exactly two importers. The surface is cleanly self-contained.

**Beat E PR** (once the new door is live) deletes:

| layer | items |
|---|---|
| UI route | `/vault/[owner]/[context]/graph/{+page.svelte,+page.server.ts}` |
| UI components | `KnowledgeGraph.svelte`, `ResourcePeek.svelte`, `GraphLegend.svelte`, `ModeToggle.svelte`, `ContextWatermark.svelte` |
| UI modules | `graph/{elements,derive,styling,layout,tiers,adjacency,peek,trail,navigation}.ts` + tests |
| API | `get_subgraph` handler, `SubgraphQuery`, `routes.rs:95`, the OpenAPI path/schema + assertion |
| services | `aggregator_subgraph`, `AggregatorSubgraphParams`, `fetch_subgraph_nodes`, `fetch_subgraph_edges` |
| types | `SubgraphResponse`, `GraphNode`, `GraphEdge`, `is_aggregator` (keep `GraphEdgeRow`, `EdgeKind`, `Polarity` — used elsewhere) |
| tests | `crates/temper-api/tests/graph_subgraph_test.rs` |
| deps | `cytoscape`, `cytoscape-fcose`, `@types/cytoscape` |

**A later PR**, after that code has deployed to every target, drops the SQL:
`graph_subgraph_nodes`, plus the dead team-graph trio `team_viewable_by`, `team_child_zones`,
`team_descendants`.

**Why split.** `graph_subgraph_nodes` is still called by shipped code. Dropping it in the same
release that stops calling it reopens the migrate-ahead-of-deploy hazard: temperkb.io and
self-hosted are independent Vercel projects on their own cadence, so an instance mid-deploy would
500 on a missing function. Stop calling it everywhere first; drop it later. The trio carries no such
risk (zero callers, ever) but rides the same migration for tidiness.

New migrations must number **above `20260709000005`** (a sibling session landed that file today).

### D10 — The team-graph trio is dropped, not wired

`team_viewable_by` / `team_child_zones` / `team_descendants` were born in the Atlas R1 migration
(`20260703000002_team_graph_scope_reads.sql`) for descendant-zone enumeration, retained by PR #324's
chunk-9 disposition pending a Beat E decision because the Beat 2a spec named an unbuilt
`TeamZoneMark`.

Nothing in this design needs zone enumeration: the context view addresses **one** context by ref, and
team scoping already flows through the live `resources_in_team_scope` (seven Atlas SQL callers).
Beat E is the last beat of the reshape — if zones are not needed now, they are not needed. Dropping
also retires `team_descendants`' `is_active` soft-delete gap rather than carrying it; the audit
prohibits reviving it as-is.

---

## 4. Architecture

### 4.1 Backend

The node half needs **no new function**: `graph_atlas_nodes_visible(p_profile, p_ids)` is already
generic — arbitrary id set, gated through `resources_visible_to` (deny-as-absence), and it computes
`home = 'cogmap' | 'context'` itself, which is what drives the mark shape.

New SQL (mirroring `graph_region_composition_edges` from
`migrations/20260708000002_graph_region_composition.sql`, whose full edge-visibility predicate is
reproduced conjunct-for-conjunct — both endpoints in `resources_visible_to`, `NOT is_folded`, and
`anchor_readable_by_profile` on the edge's home anchor):

```sql
graph_context_containers(p_profile uuid, p_context_id uuid,
                         p_container_types text[], p_depth int)
  RETURNS TABLE(id uuid, label text, member_count int)

graph_context_residual_counts(p_profile uuid, p_context_id uuid, p_group_key text,
                              p_container_types text[], p_depth int)
  RETURNS TABLE(group_value text, count int)

graph_context_composition_edges(p_profile uuid, p_seed_ids uuid[], p_depth int)
  RETURNS TABLE(id, source_id, target_id, edge_kind, polarity, label, weight)

graph_context_residual_members(p_profile uuid, p_context_id uuid,
                               p_group_key text, p_group_value text,
                               p_container_types text[], p_depth int)
  RETURNS TABLE(id uuid)
```

Plus the `DROP`+`CREATE` migration widening `graph_atlas_nodes_visible` with `stage` (D8).

All SQL lives in the persistence layer; a new `context_graph_service` in `temper-services` composes
them. No `sqlx::query!()` inline in any surface. Test-target macro queries need
`cargo make prepare-services` / `prepare-api`.

### 4.2 Wire types

`temper-core` with `ts-rs` derives, per the shared-types rule. `AtlasSubgraph` is **reused unchanged**
for the drill (Beat D set this precedent) apart from `AtlasNode.stage: Option<String>`.

```rust
pub struct ContextPanorama {
    // Container territories carry `TerritoryKind::Context`, not a new variant: `kind` selects the
    // TINT, and per D6 tint encodes the AXIS. A goal container sits on the builder axis, so it is
    // `Context`-tinted even though it is rooted at a goal. `label` carries the goal's title.
    pub containers: Vec<Territory>,
    pub residual: ResidualGroups,
    pub group_keys: Vec<GroupKeyMeta>,   // what else you could group by
}
pub struct ResidualGroups { pub group_key: String, pub buckets: Vec<ResidualBucket> }
pub struct ResidualBucket { pub value: String, pub count: i32 }
pub struct GroupKeyMeta { pub key: String, pub distinct_values: i32, pub coverage: i32 }
```

Typed structs, never `serde_json::json!()`.

### 4.3 Endpoints

```
GET /api/graph/contexts/panorama?context_ref=@me/temper&group_by=doc_type       → ContextPanorama
GET /api/graph/contexts/composition?context_ref=…&container=<uuid>&depth=1      → AtlasSubgraph
GET /api/graph/contexts/composition?context_ref=…&group=doc_type:session&depth=1 → AtlasSubgraph
```

The context arrives as a **query param, not a path segment**: a decorated ref is `owner/slug` and
contains a slash. This mirrors the existing `GET /api/graph/subgraph?context_ref=…`, and reuses
`parse_context_ref` (`temper-core/src/context_ref.rs:87`) → `resolve_context_ref`
(`temper-services/src/services/context_service.rs:101`), which already carries the right error
taxonomy: id/handle miss → `NotFound`, team non-membership → `Forbidden` **without leaking the
context's existence**.

Thin handlers: middleware → typed extractor → service → response. Auth before any read; every query
scopes through `resources_visible_to`.

### 4.4 Frontend

| file | change |
|---|---|
| `nav.ts` | `?context=<slug>` scope param; `Focus` gains `{kind:'container'; id}` and `{kind:'bucket'; groupKey; value}`; `deriveTier` maps both to 1; `buildContextUrl`, `buildDrillContainerUrl`, `buildDrillBucketUrl`. A container id is a *resource* uuid, so it does **not** reuse the `territory:` token — territory ids are ephemeral region ids and `territoryIds()` splits them on `~` |
| `AtlasCanvas.svelte` | first branch becomes `!cogmapId && !contextSlug && home` — otherwise a context door falls into `TierHome` |
| `TierPanorama.svelte` | render the residual tray (new); containers already work post-`ad324b09` |
| `TierNeighborhood.svelte` | pass `coreHome` through to `forceNeighborhood` |
| `forceNeighborhood.ts` | `coreHome` param on the radial (D5) |
| `graph-reads.ts` | `readContextPanorama`, `readContextComposition` + path builders |
| `+page.server.ts` | `?context=` branch |
| `crumbModel.ts` | context + bucket crumb segments |
| `TierHome.svelte:149` | stop hand-rolling `/vault/<owner>/<slug>`; route through `vault-url.ts` |
| `HomeA11yList.svelte:92` | **bug:** links `/vault/<owner_ref>`, dropping the context slug entirely — the a11y mirror of the build circle never reaches the context. Point it at `contextGraphHref(owner_ref, slug)`, per north-star Decision 9 (the list is the *equivalent* of the field) |
| `CompositionA11yList.svelte` | reused as-is (Ideas/Sources via `groupByAxis`) |

### 4.5 Error handling

- Unknown/invisible context ref → 404, deny-as-absence (never "exists but forbidden").
- Empty context (no containers, no residuals) → the existing `emptyMessage` path, not a blank canvas.
- Container drill returning zero nodes → `AtlasCanvas`'s `hasNeighbors` guard already covers it.
- Ephemeral/stale ids → redirect to the panorama rather than 500, as `compositionOrPanorama` does.

---

## 5. Testing

- **Pure TS:** `territoryWeight` (`log1p` ramp, `0 → 0`, and the *region with null salience but
  member_count > 0* case — a behaviour change in `ad324b09` that is currently uncovered);
  `nav.ts` context/bucket tokens; `forceNeighborhood` radial with `coreHome: 'context'` (mirror of
  `forceNeighborhood.radial.test.ts`); residual-tray model.
- **`/dev/atlas` harness:** commit sanitized `contextPanorama` + `contextDrill` scenarios;
  `fixtures.test.ts` pins the scenario list and the `AtlasViewData` key set.
- **DB (`test-db`):** each new SQL function against a real Postgres, including the **deny direction** —
  a resource visible to A but not B must be absent from B's panorama counts *and* drill, and the
  residual counts must not leak the existence of invisible resources.
- **e2e:** the door round-trip (left-nav link and build circle resolve to the same URL and view).

Run `cargo make test-e2e-embed` if any context/wire/ingest path is touched; `test-e2e` alone skips
the embed-gated tests.

---

## 6. Out of scope

### Rejected (load-bearing — these were considered and turned down)

- **All 103 aggregators as containers.** 33 empty circles; revives the inert-sphere defect. (§2)
- **Residuals in the force field.** Flattens the survey; `Maintenance` 1.00 → 0.16. (D2)
- **A single "Unfiled" bucket.** A 644-node hairball with no way in. (D2/D3)
- **Gold territories.** Would make the context panorama visually identical to the cogmap panorama and
  destroy temperature-as-axis. (D6)
- **A hardcoded doc-type list for residuals.** Bakes today's schema into the model. (D3)
- **Excluding sessions by rule.** No `doc_type <> 'session'` anywhere; the group-by produces them,
  `filters.docTypes` hides them. (D3)
- **Dropping the SQL in the Beat E PR.** Deploy-skew hazard. (D9)
- **Flipping `nodeMarkShape`.** The visual language must not change meaning between views. (D5)

### Deferred (sensible, just later)

- `TeamZoneMark` / descendant-zone enumeration — trio dropped; rebuild from scratch if ever wanted.
- Grouping the residual tray by keys other than `doc_type` — the payload carries `group_keys` so the
  UI can offer it; the picker itself is not built.
- Multi-container union drill (Beat D's shift-select) for containers.
- The interstitial's WS3 list/table rebuild — a *sibling* surface. The context view is the spatial
  Composition surface; the list view remains the reading/filtering surface. They coexist.

---

## 7. Open questions

- Does the residual tray belong inside the `AtlasCanvas` SVG (scales/pans with the camera) or in the
  page chrome beside the legend (fixed, always reachable)? Leaning chrome — a doorway should not pan
  away. Settle against the harness.
- `graph_context_containers` depth: the numbers above use depth 2. Depth 1 (`parent_of` only) yields
  tighter, more literal containers. Settle against the harness with both.

---

## 8. Connections

- Task `019f420d` (Beat E) · goal `019f28a1` (Graph Atlas) · north star `019f39ca`.
- Beat D spec `2026-07-08-atlas-beat-d-region-resources-drill-spec.md` — the drill this reuses.
- Interstitial `019f420c` — WS1+WS2 shipped as PR #312; WS3 (list/table) remains, and is a sibling
  of this surface, not a dependency.
- PR #324 chunk 9 — the team-graph trio disposition this spec closes.
- `019f298b` — `fetch_subgraph_edges` visibility leak, already `done`; not a reason to retire.
- `019f468b` — `--goal` filter `parent_of`/`advances` divergence; fixed in `586717c2`.
- Prototype commits on `jct/atlas-beat-e`: `ad324b09` (kind-agnostic panorama + `log1p`),
  `4e0e83d6` (context tint + temperature-as-axis).
