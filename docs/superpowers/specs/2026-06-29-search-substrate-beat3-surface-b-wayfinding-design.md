# Search substrate — Beat 3 / Surface B: cognitive-map wayfinding (design)

**Status:** design-only. Build is handed to the WS7 cognitive-map agent-invocation arc.
**Date:** 2026-06-29
**Goal:** `019f040c` — *Search substrate: general search (FTS+vector+graph) + cogmap wayfinding*. Beats 1–2
(Surface A: stored tsvector + GIN, and the FTS+vector+graph blend) are shipped (#179, #183). This is the
standalone Beat 3 deliverable: the Surface B design + the handoff into the cogmap arc.

## Sibling specs (the substrate this builds on)

- `2026-06-26-search-substrate-beat1-stored-tsvector-design.md` — stored `kb_resource_search_index` + GIN.
- `2026-06-26-search-substrate-beat2-surface-a-design.md` — the `unified_search` blend core (FTS ∪ vector ∪
  graph → weighted-sum fusion → order). **Surface B reuses this core unchanged.**
- `2026-06-25-cognitive-map-agent-invocation-architecture-design.md` — the steward/invocation world Surface B
  serves (a steward is invoked *scoped to a cogmap*, reads its telos, judges within the map's visibility scope).
- `2026-06-28-cogmap-analytics-read-surface-design.md` — the region-metrics/analytics reads that already
  expose `salience` and its components.

---

## 1. Throughline

Surface A (`/api/search`, Beats 1–2) made general search *use the substrate*: it blends FTS + vector + graph
and ranks. Its corpus is **context-homed** resources visible via `resources_visible_to`.

The substrate, though, has a second home class. `kb_resource_homes.anchor_table ∈ {'kb_contexts','kb_cogmaps'}`
(canonical_schema.sql:276–285) makes a **cognitive map a peer resource-home to a context**. Surface B is what
search looks like over *that* home class — and, more broadly, what the whole workflow surface looks like when a
steward agent works *inside* a cogmap instead of inside a context.

## 2. The unifying insight (the defining decision)

**The FTS+vector+graph engine is scope-agnostic.** Once a bounding set of `resource_id`s is established, the
`unified_search` blend runs identically. Surface A and Surface B differ in exactly **one** thing: *how the
bounding scope is established.*

| | Scope establishment | Corpus |
|---|---|---|
| **Surface A** (`--context`) | direct | context-homed resources visible to the principal |
| **Surface B** (`--cogmap` / `--wayfind`) | lens-driven first pass | cogmap-homed participants the principal can see |

This is why the goal insists "two surfaces, not one blended model" — yet they share the **same back half**.
Surface B is a *scope-resolution front end* in front of the Beat-2 blend, never a second ranking model.

The design therefore has two coordinated halves: cogmap-as-home addressing (Half 1) and the lens-driven
discovery pass (Half 2). Half 1 is also the graceful degradation of Half 2 (§5).

---

## 3. Half 1 — Cogmap as a first-class resource home (`--cogmap` symmetric plumbing)

A steward (or any authorized agent) working in a cogmap uses the **same workflow verbs** it already knows,
swapping the *home selector*, not the tool:

| Verb | Context form (today) | Cogmap form (new) | Effect |
|---|---|---|---|
| create | `temper resource create --context @me/foo` | `temper resource create --cogmap <ref>` | writes the resource + a `kb_resource_homes` row with `anchor_table='kb_cogmaps', anchor_id=<cogmap>` |
| edge | `temper edge assert … --context @me/foo` | `temper edge assert … --cogmap <ref>` | edge authored within the map; endpoints are cogmap-homed |
| search | `temper search … --context @me/foo` | `temper search … --cogmap <ref>` | single-map scope (§5) → shared blend |

`--cogmap <ref>` resolves by the standard trailing-UUID ref rule (a cogmap ref is a bare cogmap UUID or a
decorated `slug-<uuid>`). The agent learns **a home flag, not a new tool**.

### Relationship to the "ingest → fold" model

The goal notes that context-homed resources are "sources of ingest a steward folds into a cogmap." That path
(fold an *external* context resource into a map) is real and unchanged. Half 1 adds the *complementary* path:
**direct authorship into a map** — a steward creating concept resources and edges that are cogmap-homed from
birth. Both coexist; direct authorship is what a steward does *inside* the map it stewards. No precedence
conflict: a resource has exactly one home (UNIQUE on `kb_resource_homes.resource_id`), so a resource is either
context-homed or cogmap-homed, never both.

### Write-path authorization

Cogmap-homed writes gate on the **producer axis**, not the consumer axis. A principal may write into a cogmap
iff that cogmap is one they can act in (steward/author standing). The read predicate `cogmap_readable_by_profile`
(canonical_functions.sql:259–267) establishes *visibility*; the write path additionally requires authoring
standing on the map. The exact write predicate is **deferred to the cogmap arc's RBAC** (it composes with the
steward-invocation scoping in the invocation-architecture spec); this spec only requires that the `--cogmap`
write path route through a producer-side check before the home row is written ("auth before writes").

---

## 4. Half 2 — `--wayfind`: the lens-driven discovery pass

When the agent does **not** name a single map, `temper search --wayfind [--lens …] [--regions N] <query>` runs the
four-step first pass, then the shared blend:

1. **Lens** — default global cogmap lens (`kb_cogmap_lenses` with `cogmap_id IS NULL`), or an overridable lens
   set via `--lens`. The lens carries both the affinity weights and the **salience weights** `s_telos`,
   `s_ref`, `s_central` (canonical_schema.sql:684–700; `Lens::telos_default()` = `s_telos 0.5 / s_ref 0.3 /
   s_central 0.2`, affinity.rs:72–84).
2. **Authz** — `cogmap_readable_by_profile(principal, cogmap)` → the set of maps this principal can see
   (membership-flat: direct `kb_team_members ∩ kb_team_cogmaps`; the L0 kernel `system-default` is always in
   this set because every approved principal is a member of `temper-system`).
3. **Region salience trace** — across those maps, score regions (§4.1), take the top-`N` (`--regions N`, default
   in §8). `--regions 3` means "results within the 3 top-scoring regions."
4. **Scope + blend** — collect those regions' members (`kb_cogmap_region_members`, visibility-gated per member
   through `resources_visible_to` — never returned wholesale, schema 746–747), forming the bounding
   `resource_id` set, and run the **existing `unified_search` blend** over it. Surface B passes this set as the
   blend's corpus filter; everything downstream (FTS, vector ANN, graph expansion, fusion, order) is Beat-2 code.

### 4.1 Region selection — the weighted funnel (the central ranking decision)

Region selection is **haystack reduction**: keep regions that are *both interesting and relevant*, so the
precise blend runs over high-value scope rather than the whole map. The score blends two signals:

```
region_score = α · salience_norm  +  β · query_centroid_cosine
```

- **`salience`** — lens-aware region importance. Default path uses the memoized `kb_cogmap_regions.salience`
  (computed under the region's own `lens_id`). Under an **overridden** lens, salience is **recomputed from the
  stored components** `telos_alignment` / `reference_standing` / `centrality` (canonical_schema.sql:725–744)
  under the override's `s_*` weights — so cross-region comparison stays coherent within the chosen lens.
- **`salience_norm`** — salience **normalized within the candidate region pool** (per-lens min-max or z-score
  over the regions in scope) **before** blending. This is load-bearing: raw salience scales with member count
  and edge mass, so a 500-member map's regions would otherwise dominate a 12-member map's regions on magnitude
  alone. Normalization makes a sparse map compete fairly (§4.2).
- **`query_centroid_cosine`** — cosine of the query embedding against `kb_cogmap_regions.centroid` (a stored
  `vector(768)`, the pool-per-concept-then-mean of members). Computed by a **per-map sequential scan** — there is
  **no HNSW index on the centroid** (schema 722–724, OQ-2 deferred), but region counts are clustering outputs
  (tens, not millions), so a scan is cheap. No new index is required for Beat 3.

**Why a genuine weighted blend, not lexicographic "salience rank, cosine tiebreak":** a lexicographic order can
never let a high-relevance, low-importance region into the top-N — which is exactly the sparse-but-meaningful
region (§4.2). The weighted blend lets relevance *buy* a top-N slot.

**Weights are SQL-resident and corpus-tuned.** `α`, `β`, and the recall-floor knob live as constants in the
Surface-B SQL function's leading CTE — the same single-home convention as `unified_search`'s `k` CTE
(beat2 sql:97–98). They are **not** API/CLI params and **not** pre-guessed; defaults are conservative and
calibrated on the real corpus (§8). The salience term suppresses high-text-match / low-value regions; the cosine
term (and the recall floor) protects relevant ones.

### 4.2 Sparsity is the lifecycle norm, not an edge case

Two structural facts make sparsity central, not incidental:

- **`centrality` scales with `member_count` × internal declared-edge mass** (cogmap_region_centrality,
  canonical_functions.sql:488–503) and **`reference_standing` aggregates reinforce-counts**
  (cogmap_region_reference_standing, 476–483). Both **penalize sparsity by construction** — a sparse region
  starts with a low salience floor.
- A **sparse region can be permanently important** (a small, rarely-touched, high-telos-alignment cluster).

The weighted blend + normalization (§4.1) is the first defense: `query_centroid_cosine` is actually a *sharp*
signal for sparse regions (few members → centroid ≈ the member), so a relevant sparse region scores high on `β`
even with a low `salience_norm`, and the normalization stops big maps from crowding it out.

**Recall-floor lane (knob, default conservative).** Optionally always admit the single best
`query_centroid_cosine` region regardless of salience — the cluster-search "always probe the nearest cluster"
guarantee — so a sparse-but-perfect region is never excluded *purely* on importance. **Spec the knob, default it
off, enable only if corpus eval shows sparse-region misses.** This is the YAGNI-candidate: wire it, don't lean on
it until measurement demands it.

---

## 5. Cold-start & graceful degradation — regions are an optimization, not a precondition

A freshly-born cogmap has **no regions, no components, no members** until a later `MaterializeCogmapShape`
(`_project_cogmap_seeded`, canonical_functions.sql:677–699; L0 is born content-light with empty `blocks=[]`).
"No regions yet" is therefore **every map's first chapter**, and the funnel must degrade, never error.

The two halves **nest**: Half 1's single-map scope *is* the degenerate Half 2 with the region stage bypassed.

| Map state | Wayfind behavior |
|---|---|
| rich regions | region-salience funnel (§4) → top-N regions → members → blend |
| zero / thin regions (forming, or permanently sparse map) | **bypass the region stage** → scope over the map's homed participants directly (`resources_accessible_to_cogmap` / the home-anchor set) → blend |
| `--regions N` against a region-less map | transparently widens to the whole map; not an error |
| `--cogmap <ref>` (Half 1, explicit single map) | always the direct homed-participant scope — i.e. the bypass path, by construction |

The "thin" threshold (how few regions/members trips the bypass) is a tuning constant (§8), defaulted so that a
map with no usable regions always falls through to direct-scope. The degradation is *silent and correct*: the
agent gets results from a forming map exactly as it will from a mature one, just with a coarser scope.

---

## 6. Surface shape (CLI / API / MCP)

**One verb, one endpoint, three scope-resolution paths feeding the shared blend.**

- **CLI** — `temper search` stays the verb. `--context @me/foo` | `--cogmap <ref>` | `--wayfind` select the
  scope-resolution path; `--lens <ref…>` and `--regions <N>` modify wayfind. `--cogmap <ref>` is *also* the
  home-selector on `resource create` and `edge assert` (Half 1).
- **API** — the search service resolves scope via one of three paths
  (`context-scope` | `single-cogmap-scope` | `wayfind-scope`) and passes the resulting `resource_id` set into the
  **existing `unified_search` blend** as a corpus filter. No second blend function — the candidate→fusion→order
  logic is reused verbatim. The one additive change to the blend is a **scope-id-set filter param** (`p_scope_ids
  uuid[]`) that generalizes the dormant `p_context_id` EXISTS-filter into "restrict the corpus to this explicit
  id set"; Surface A keeps using `p_context_id`, Surface B uses `p_scope_ids`. New gated SQL is otherwise limited
  to the **scope-resolution functions** (visible-maps, region-rank, region-members→ids, direct-homed-ids).
- **MCP** — mirrors the CLI: the existing search tool gains the scope discriminator; the create/edge tools gain
  the `--cogmap` home selector. (MCP enum params inline per `project_mcp_enum_params_must_inline`.)

This honors both "works the same way" (one verb, one engine, symmetric to `--context`) and "two surfaces"
(distinct scope-resolution).

---

## 7. Visibility & auth — gated at every stage ("no view from nowhere")

Defense-in-depth, matching the existing cogmap reads (deny → zero rows, never an error):

1. **Map admission** — `cogmap_readable_by_profile` (membership-flat; differs from the ancestor-expanded
   resource reachability — a principal sees a map only via direct `kb_team_members ∩ kb_team_cogmaps`).
2. **Region read** — same gate the analytics surface uses; folded regions excluded
   (`idx_kb_cogmap_regions_map … WHERE NOT is_folded`).
3. **Member dereference** — every member id resolved through `resources_visible_to` (or
   `resources_readable_by` for a cogmap principal); members are **never returned wholesale** (schema 746–747).
   A member the principal cannot see is silently dropped from scope.
4. **Blend** — `unified_search` already enforces `resources_visible_to` *inside* each candidate function, so even
   if scope resolution over-collected, the blend re-gates. Belt and suspenders.

No new visibility primitive is introduced; Surface B *composes* the existing ones.

---

## 8. Tuning constants & open questions (resolve during build, on the real corpus)

All constants live in the Surface-B scope-resolution SQL (single home, mirroring `unified_search`'s `k` CTE):

- `α` (salience weight) / `β` (query-cosine weight) in `region_score`.
- Recall-floor on/off (§4.2), default **off**.
- Default `--regions N` and the per-call ceiling on `N`.
- The "thin map" bypass threshold (§5).
- Salience normalization method (min-max vs z-score within the candidate pool).

Open questions carried from the goal, now made concrete:

- **Multi-map aggregation** — the recommended model pools regions across *all* visible maps and ranks at the
  region grain (region salience already encodes map-level richness through its components), rather than ranking
  maps first then regions. Validate this is right vs. a two-level (map then region) selection on the real corpus.
- **Lens override semantics** — confirm recompute-from-components under an override lens matches operator
  intuition vs. just re-filtering to regions whose `lens_id` already equals the override.

---

## 9. Validation / acceptance (for the build in the cogmap arc)

- `--cogmap <ref>` create/edge writes a `kb_resource_homes` row with `anchor_table='kb_cogmaps'`; the resource is
  invisible to context search (Surface A) and visible to `--cogmap` search of that map.
- `temper search --cogmap <ref>` returns the same ranked shape as Surface A, scoped to the map's homed
  participants; results gate correctly (a principal who can't see the map gets zero rows, not an error).
- `--wayfind` over a principal with several visible maps scopes into the top-`N` regions and ranks within; a
  sparse-but-relevant region surfaces (regression test: a thin high-cosine region beats a large low-cosine one).
- **Cold-start**: `--wayfind` / `--cogmap` against a region-less (freshly born) map returns blend results over
  the whole map, never an error; `--regions N` widens silently.
- All scope-resolution SQL gated; member dereference visibility-gated. Tests green under
  `cargo make test-artifacts` (the Embed CI tier, where cogmap/ONNX tests run).

## 10. Scope boundaries

- **In scope (this spec):** the Surface B design — cogmap-as-home addressing, the wayfind funnel, the region
  selection model, cold-start degradation, surface shape, gating.
- **Out of scope / deferred to the cogmap arc's RBAC:** the exact producer-side **write** predicate for
  `--cogmap` authorship (composes with steward-invocation scoping).
- **Explicitly not built here:** this is design-only. Implementation is handed to the WS7 cognitive-map
  agent-invocation arc, where wayfinding belongs (orientation aligned with the L0 kernel / `cogmap_genesis` /
  steward world), not the `/api/search` substrate.
- **No new index** required (region count is small; centroid scan is cheap). If eval later shows the centroid
  scan is a bottleneck on very large maps, a centroid HNSW (schema OQ-2) is the additive follow-up.
