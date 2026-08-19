# Graph Atlas — Atlas Home design spec

**Date:** 2026-07-04
**Goal:** `graph-atlas-visualization` (`019f28a1-03f2-7aa1-a367-f6f8db8b0e7f`)
**Task:** `019f2aaf-5a88-7ff2-8bba-7b94fa34234e` (mode=plan, effort=large)
**Depends on:** Chunk C2 (shipped, PR #258) — the `you → teams` home on the AtlasCanvas substrate
**Sequenced with:** C3 (chrome) is an independent sibling; Chunk D (retire old route) is last.

## 1. What Atlas Home delivers

Grow the C2 canonical `@me` home from its `you → teams` half into the **full membership
graph** — `you → teams → cogmaps` — with cogmaps as first-class **enterable doors**, and
add the interior view that a cogmap door opens onto: a **cogmap-scoped panorama** rendering
that cogmap's own regions and facets, team-independent.

Split out of C2 because it carries **net-new backend reads** (C2 was frontend-only) and
deserves its own review arc.

## 2. Settled decisions (2026-07-03 brainstorm — restated, not re-litigated)

- **Model = membership graph.** Teams and cogmaps are rendered as the membership graph
  itself; team→cogmap edges are `kb_team_cogmaps` rows. That table is **many-to-many** (a
  cogmap belongs to 0..N teams) and a profile participates in a cogmap *through* a team — so
  a shared cogmap shows **multiple** team edges (no hierarchy), and we do **not** flatten to a
  wall of cogmaps (that would drop the "reached through a team" relationship).
- **Enter-a-cogmap = the cogmap as its own place.** Clicking a cogmap door shows *that
  cogmap's* interior directly, team-independent. Forced by the data: a cogmap has 0..N teams
  and may have no region, so "shortcut into its team" is ill-defined.

## 3. Grounded substrate (verified 2026-07-04)

What already exists and is reused as-is:

| Piece | Location | Reused for |
|---|---|---|
| `cogmap_visible_maps(profile) → SETOF uuid` | `migrations/20260701000002_cogmap_read_up_flip.sql:53` | Which cogmaps a profile may see (home read) |
| `cogmap_readable_by_profile(profile, cogmap) → bool` | `…20260701000002:35` | Per-cogmap gate (panorama read) |
| `kb_cogmaps(id, name, telos_resource_id, …)` | `…20260624000001_canonical_schema.sql:243` | Cogmap door names + counts |
| `kb_team_cogmaps(cogmap_id, team_id) PK` | `…20260624000001:254` | team→cogmap membership edges |
| `kb_cogmap_regions(id, cogmap_id, lens_id, label, member_count, salience, is_folded, …)` | `…20260624000001:725` | Cogmap-scoped territories |
| `resources_in_team_scope(profile, team)` + count idiom | `…20260703000002_team_graph_scope_reads.sql:53` / `team_service.rs:537` | Per-team resource count |
| R2 `territory_overview` → `TerritoryOverview` | `graph_service.rs:346` / `graph_territory.rs:74` | **Wire type + renderer reused by the cogmap panorama** |
| R3 `territory_slice(profile, region_id)` (region-keyed, cogmap-gated) | `graph_service.rs:455` | **Region drill *within* a cogmap — works unchanged** |
| `TierPanorama.svelte`, `TierHome.svelte`, `homeLayout.ts` | `src/lib/components/graph/atlas/`, `src/lib/graph/atlas/layout/` | Frontend extension points |
| server reads + load fn `!teamId` branch | `src/lib/server/graph-reads.ts`, `…/graph/[owner]/+page.server.ts:21` | Where new reads wire in |

**Why the existing R2 functions cannot just be re-pointed at a cogmap:** they are
team-parameterized — `graph_region_territories(profile, team, lens)` joins
`kb_team_cogmaps ⋈ team_ancestors(team)`, and orphans/bridges scope through
`resources_in_team_scope(profile, team)`. A cogmap panorama is scoped by a *single cogmap*
(gated by `cogmap_readable_by_profile`), not a team subtree. So it needs its own thin SQL
functions — but the returned **shape** is identical, so the wire type and renderer are shared.

## 4. Backend design

Three net-new pieces. All are **service-direct reads** (per CLAUDE.md: reads stay
service-direct; only writes route through `DbBackend`), following the existing
`graph_service.rs` template: a boolean visibility gate (deny-as-404-absence) then a thin
`query_as` over a `STABLE` SQL function, mapped into a ts-rs wire type.

### 4A. Home read — `GET /api/graph/home`

Returns the full `you → teams → cogmaps` bipartite membership graph **with count hints baked
in**, so the home renders from one read (no N+1, load fn stays simple).

New SQL function `graph_home_cogmaps(profile)` — the profile's visible cogmaps, each with its
team memberships (intersected with visible teams) and counts:

```sql
CREATE FUNCTION graph_home_cogmaps(p_profile uuid)
RETURNS TABLE(cogmap_id uuid, name text, team_ids uuid[], region_count int, facet_count int)
LANGUAGE sql STABLE AS $$
    WITH visible AS (SELECT cogmap_id FROM cogmap_visible_maps(p_profile) t(cogmap_id))
    SELECT c.id, c.name,
           array_agg(DISTINCT tc.team_id) FILTER (WHERE tc.team_id IS NOT NULL),
           (SELECT count(*) FROM kb_cogmap_regions r WHERE r.cogmap_id = c.id AND NOT r.is_folded)::int,
           (SELECT count(*) FROM kb_resource_homes h WHERE h.anchor_table='kb_cogmaps' AND h.anchor_id = c.id)::int
    FROM visible v
    JOIN kb_cogmaps c ON c.id = v.cogmap_id
    LEFT JOIN kb_team_cogmaps tc ON tc.cogmap_id = c.id
    GROUP BY c.id, c.name;
$$;
```

Team counts extend the existing teams path. Rather than change the shared `GET /api/teams`
(`TeamRow` is used elsewhere), the home read composes teams itself: reuse
`resources_in_team_scope(profile, team)` count for `resource_count` and
`count(*) FROM kb_team_cogmaps WHERE team_id = $1` for `cogmap_count`, over the caller's
visible teams (`list_teams` set).

**New wire types** (`crates/temper-core/src/types/graph_home.rs`, `export_to = "graph_home.ts"`):

```rust
pub struct HomeTeam   { pub id: Uuid, pub slug: String, pub name: String,
                        pub resource_count: i32, pub cogmap_count: i32 }
pub struct HomeCogmap { pub id: Uuid, pub name: String, pub team_ids: Vec<Uuid>,
                        pub region_count: i32, pub facet_count: i32 }
pub struct AtlasHome  { pub teams: Vec<HomeTeam>, pub cogmaps: Vec<HomeCogmap> }
```

`team_ids` on each cogmap **are** the bipartite edges — the frontend draws one edge per
`(team, cogmap)` pair. `HomeCogmap.name` also resolves the C2 "cogmap · N facets" generic
label (see §6, D3).

### 4B. Cogmap-scoped panorama — `GET /api/graph/cogmaps/{id}/panorama`

The interior a cogmap door opens onto. Gate: `cogmap_readable_by_profile(profile, cogmap)`
→ 404 on deny (absence). Returns the **existing `TerritoryOverview`** shape (territories =
this cogmap's live regions; orphan_nodes = cogmap-homed resources with no region; bridges =
visible edges between differently-regioned members). Optional `?lens_id=`; default = the
cogmap's primary lens (see §6, D2).

New SQL, mirroring R2 but keyed on cogmap:

```sql
CREATE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id, reg.label, reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;

CREATE FUNCTION graph_cogmap_orphan_nodes(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid) …
-- cogmap-homed resources gated per-row by resources_visible_to(profile);
-- ranked by edges_visible_to degree; only when this cogmap has no live region for them.
```

Because the shape is `TerritoryOverview`, the frontend routes the response straight into the
**unchanged `TierPanorama.svelte`**. **Region drill within the cogmap reuses R3
`territory_slice` unchanged** (it is region-keyed and cogmap-gated) → `TierTerritory.svelte`.

## 5. Frontend design

### 5A. `homeLayout.ts` — three-column extension

The file's own docstring already anticipates this ("the Atlas Home chunk grows a third
(cogmap) column onto this same shape"). Changes, all pure:

- `HomeNode.kind` gains `'cogmap'`.
- `layoutHome(teams, cogmaps, size)` — second arg. Columns: `you` at `x=0.16·W`, teams at
  `0.52·W`, cogmaps at `0.86·W` (nudge team column left from C2's `0.58` to make room).
- Team→cogmap edges: for each `HomeCogmap`, one `HomeEdge` per `team_id` that is present in
  the laid-out team set. `HomeEdge` is positional-only, so no schema change.
- Cogmaps with no visible team edge (possible via explicit access-grant, no team) sit in the
  column with only the implicit membership; render but with no left edge.
- Extend `homeLayout.test.ts`: column placement, edge count = Σ visible `team_ids`, a shared
  cogmap producing 2 edges, zero-cogmap and zero-team edge cases.

### 5B. `TierHome.svelte` — cogmap doors + count chips

- Render the cogmap column as `↵` doors mirroring team doors; show `region_count` /
  `facet_count` as small count chips.
- Team doors show `resource_count` / `cogmap_count` chips (net-new data from 4A).
- **Normalize door colors through `palette.ts`** — C2 hard-codes door hexes in TierHome;
  add `TEAM_DOOR` / `COGMAP_DOOR` tokens to the single-source palette and consume them (the
  palette already has `TERRITORY_TINTS.cogmap = '#e8942e'`; reuse that hue family for cogmap
  doors so the door color and the cogmap-territory wash agree).
- Cogmap door enter → `goto(buildCogmapUrl($page.url, cogmapId), {replaceState:true})`.
- **Reuse, don't fork, the door-activation handler** — and fix it once (see §6, D4): the
  C2 door uses `onclick` yet prod needs a double-click; whatever the fix, both team and
  cogmap doors share one handler.

### 5C. `nav.ts` — cogmap addressing (`?cogmap=`)

Enter-a-cogmap is team-independent, so it is a **new scope param**, sibling to `?team=`, not
a `?team=` value:

- `parseCogmap(url) → string | null` (`?cogmap=<id>`).
- `buildCogmapUrl(url, cogmapId)` — set `cogmap`, clear `team` + `focus`.
- `deriveTier` unchanged for the region-drill focus inside a cogmap (territory-focus → tier 1).
- `buildHomeUrl` also clears `cogmap`.

### 5D. Load fn (`+page.server.ts`) branches

- `!teamId && !cogmapId` (home): replace bare `listTeams` with `readAtlasHome(token)` → the
  `AtlasHome` payload feeds `TierHome`.
- `cogmapId` present: `readCogmapPanorama(token, cogmapId, focus)` → `TierPanorama` (tier 0);
  if a `territory` focus is set, also `readRegionSlice` → `TierTerritory` (tier 1), exactly as
  the team path does today.
- `AtlasCanvas.svelte` `{#if}` ladder gains a `cogmapId && territories` arm (or the existing
  `tier===0 && territories` arm is reached with a `cogmapId`-set canvas prop). No new tier
  *renderer* — the cogmap panorama **is** `TierPanorama`.

### 5E. Server reads (`graph-reads.ts`)

Add sibling path-builder + thin `apiGet` wrappers: `atlasHomePath`/`readAtlasHome`
(`GET /api/graph/home`), `cogmapPanoramaPath`/`readCogmapPanorama`
(`GET /api/graph/cogmaps/{id}/panorama[?lens_id=]`). Assert both paths in
`graph-reads.paths.test.ts`.

## 6. Decisions & scope boundaries

**In scope** (acceptance criteria): the three-column membership home with count hints;
cogmap doors that enter a cogmap-scoped panorama; region drill within a cogmap (free via R3);
new reads gated by the existing visibility predicates; sqlx-macro'd SQL with regenerated
caches; e2e coverage; browser-verify.

**Decisions taken (override if you disagree):**

- **D1 — Home is one new read (`/api/graph/home`), not an augmented `/api/teams`.** Keeps the
  shared `TeamRow`/`GET /api/teams` untouched; the home is one cohesive payload.
- **D2 — Cogmap panorama default lens = the cogmap's primary/telos lens**, with optional
  `?lens_id=` override. (R2's default is the *global* telos-default; a cogmap's own place
  should default to its own lens.) If a cogmap has multiple lenses, the first by a
  deterministic order; multi-lens switching is deferred.
- **D3 — Bundle the `OrphanNode`-name fix.** C2's sparse cogmap territory shows a generic
  "cogmap · N facets" because `OrphanNode` carries `anchor_id` but no name. `HomeCogmap.name`
  already gives the home real names; additionally add `anchor_label: Option<String>` to
  `OrphanNode` (+ a name join in `graph_orphan_salient_nodes`) so C2's Tier-0 sparse
  territory also shows the real cogmap name. Additive, small, same PR narrative.
- **D4 — Fix the door single-click-vs-double-click bug as part of the shared door handler.**
  C2 code is `onclick` but prod needs a double-click; investigate (likely first click focuses
  the `role="button"` `<g>`, activation needs a second) and land the fix once for both team
  and cogmap doors. If the fix is nontrivial/unrelated, extract to a C2 follow-up branch.

**Deferred (not rejected):**

- **Tier-2 node-neighborhood *within* a cogmap.** R4 `neighborhood_slice` is
  team-parameterized (needs `resources_in_team_scope`); a cogmap-scoped traversal is its own
  SQL surface. Enter-a-cogmap ships panorama + region-drill (Tier 0→1); facet-node Tier-2
  drill inside a cogmap is a later beat. **← This is the main product-scope fork for Cole.**
- Multi-lens switching inside a cogmap panorama (D2 picks one deterministically for now).

## 7. Test & gate plan

- **Backend:** e2e/integration for each new read behind the access tier (mirrors existing
  access-scenario coverage) — a member sees their cogmaps + counts; `cogmap_readable_by_profile`
  denies a non-member with a 404 (absence, not 403); a shared cogmap yields multiple
  `team_ids`. New SQL uses `sqlx` macros; regenerate per-crate caches
  (`prepare-services`, `prepare-api`, `prepare-e2e`) after adding SQL. Run the **embed e2e
  tier** if any touched fixture path exercises it.
- **Frontend:** vitest node-env pure-fn tests for the extended `homeLayout` and the new
  `nav.ts` builders/parsers + the new read path builders; `bun run check` 0 errors; `bun run
  test` green.
- **Wire types:** `cargo make generate-ts-types` after adding the Rust structs; commit the
  regenerated `graph_home.ts` (+ any `graph_territory.ts` delta from D3) — generated types
  ride along.
- **Browser-verify** in the authed env: home shows `you → teams → cogmaps` with a shared
  cogmap drawing two edges; count chips populate; a cogmap door enters the cogmap panorama;
  a region inside it drills to the region slice; `⌂ Atlas` returns home.

## 8. Task breakdown preview (for the plan)

1. **Backend — home read**: `graph_home_cogmaps` SQL + `AtlasHome`/`HomeTeam`/`HomeCogmap`
   wire types + `atlas_home` service fn + `GET /api/graph/home` handler/route + e2e.
2. **Backend — cogmap panorama**: `graph_cogmap_territories` / `graph_cogmap_orphan_nodes`
   SQL + `cogmap_panorama` service fn (returns `TerritoryOverview`) + route/handler + e2e;
   D3 `OrphanNode.anchor_label` bundled here.
3. **Frontend — home**: `homeLayout` three-column + tests; `TierHome` cogmap doors + count
   chips + palette door tokens; D4 door-handler fix; `readAtlasHome` wiring + load-fn branch.
4. **Frontend — enter-a-cogmap**: `nav.ts` `?cogmap=` addressing; `readCogmapPanorama`
   wiring; load-fn cogmap branch → `TierPanorama`; region drill via R3.
5. **Verify**: type regen, full gate, browser-verify walk.

Tasks 1 and 2 are backend-parallelizable; 3 depends on 1, 4 depends on 2. 5 is last.
