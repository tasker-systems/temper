# Atlas Beat C — retire the team panorama; scope-filtered Home (spec)

**Status:** implementation spec, ready for plan. Third beat of the Atlas reshape
(built after A + B; both held on `jct/atlas-reshape`).
**North star:** `docs/superpowers/specs/2026-07-06-atlas-reshape-projection-class-north-star.md` (vault research `019f39ca`).
**Builds on:** Beat A field-effect + force layout; **Beat B Home reframe** (build/research
verb-lens field, `AtlasHome { build, research }`) — both shipped, held on `jct/atlas-reshape`.
**Goal:** `019f28a1`.

---

## 0. Why C reshaped — retire, don't un-mix (the sequencing payoff)

The north star framed C as **"team panorama = contexts (the axis un-mix)"**: strip regions
out of the team panorama so it surveys one coherent axis (contexts), sized + recency-weighted,
every circle enterable. That framing assumed the team panorama **survives** as a distinct
surface.

**Beat B moved the ground.** B made Home's build lens **contexts-grain** and
**access-complete**: `graph_home_contexts` surfaces *every context the caller can read*
(personal + all teams + ancestors + grants), sized and directly navigable, tinted by
owner-scope. That is already the "team's contexts, sized" survey the north star wanted — for
*all* teams at once. A separate per-team territory-overview is now a **filtered subset** of a
survey Home already renders.

So C's correct move is **retirement, not un-mixing**: delete the team panorama surface
outright, and let Home's lenses subsume it via a **flat scope filter**. This is the reshaping
payoff B was sequenced ahead of C to unlock (B-spec §0). The result is a *simpler* Atlas — a
whole semantic-zoom tier and the axis-mix defect both disappear, and no reach is lost.

## 1. Purpose (the act this surface serves)

Same projection-class as Home — **orientation** (`(substrate, perspective, time) → small
structured survey`) — at a **narrower perspective**: one team's workspace instead of the whole
footprint. Because access already flattens the team DAG into the build union (§3), "one team's
workspace" is honestly expressed as **"the build survey, perspective-narrowed to one
owner-scope."** No new act, no new surface — a lens on an existing one.

The north star (§4.8, §5) additionally mandates this workspace survey encode **"what's alive"**:
size = magnitude, **recency = liveness**. B shipped build sized by `resource_count` alone; C
adds the recency channel so the workspace view is honest about activity (a small active context
out-signals a huge dormant one).

## 2. The decision — retire the team panorama

The `?team`-scoped Tier-0 territory-overview is **deleted**. "Team X's workspace" becomes
**Home build filtered to one owner-scope** (`?home=build&scope=+tasker`); its cogmap doors
become **Home research filtered the same way**.

### 2.1 Why retirement loses no reach (the DAG argument)

Two different DAG relationships exist; only one matters for orientation, and Home already
serves it:

- **Upward read-access** (member of child team → sees ancestor resources). This is the real
  access substrate, and **B's build read already flattens it**: `graph_home_contexts`' candidate
  UNION (self-owned · team-member-owned · shared-into-your-team · read-grant), gated by
  `context_visible_to` (= you + ancestors), surfaces *every context you can read, wherever it
  sits in the DAG*. The build union is **access-complete**.
- **Downward zone-existence** (Chunk A's net-new descendant enumeration): see a child team you
  are *not* a member of, as an inert existence marker you cannot enter.

Retiring the team panorama drops **only** the downward zone-existence affordance — knowing an
inaccessible sibling/child team *exists*. That is **org-topology** (a boundary-sensing/structure
act), which the north star deliberately did **not** prioritize for the workspace survey (§7
deferred the team-grain aggregate; org-chart is the same family). It belongs to
team-management/admin UI, not the Atlas contexts survey. Every *workable* context stays in the
build field.

### 2.2 Consequence — the scope filter is flat

Because access already flattened the DAG, the scope filter needs **no nested scope-tree**. It is
a flat set of the distinct owner-scopes present in the caller's reachable set (`@me`, `+tasker`,
`+temper`, …). Chunk A's `TeamZone` / zone-rendering path is deleted; the backend
descendant-enumeration semantics stay **dormant** (see §9 — kept, not deleted).

## 3. The scope-filter interaction (the one new surface element)

Nests under B's Home state machine as a **third step after commit** — never at rest.

- **Rest** (union haze, both lenses) → **commit a lens** (`?home=build`, crisp all-scopes field)
  → **optionally narrow scope** (`?home=build&scope=+tasker`).
- **Scope chips appear only once a lens is committed** — a horizontal chip-row between the
  verb-CTAs and the field. Each chip renders in **its owner-scope tint-band color** (visual
  continuity with the body tints B computes; the legend *is* the filter). Default = **"All"**
  (no `?scope` param).
- **Chips are derived from the bodies actually present** in the committed lens — build offers
  `@me` + each team; research offers `temper` (universal) + each team. The asymmetry (build has
  `@me`; research has universal, no `@me`) is honest — derive from the real body set, never a
  hardcoded list.
- Clicking a chip narrows the field to that scope; clicking the active chip / "All" clears back
  to the union. **Back** is a real history step (`?scope` set with pushState, matching the
  `?home` convention).
- **Recency channel:** in the (narrowed or union) build field, bodies encode **size =
  `resource_count`, glow/brightness = recency** — two independent visual channels on
  `TerritoryCircle`. Research bodies keep `region_count` sizing, **no recency**.

> **Visual specifics are harness-locked, not spec-locked.** Exact chip treatment (chip vs pill
> vs underlined tab), placement, animation, and the recency→glow curve are locked on
> `/dev/atlas` against production-cleaned fixtures during the plan's **Task 1 spike**
> (`[[feedback_local_proddata_render_harness_for_ui]]`), exactly as Beat B's specifics were. The
> decisions above are the contract the spike refines, not overrides.

## 4. Data contract — minimal backend

**Scope-filtering is pure client-side.** The build read **already returns all scopes**, so
narrowing to one `owner_ref` is an array filter in the loader/component — **no new SQL function,
no new read, no new wire type for the filter itself.**

**The only backend change is the recency enrichment.** `graph_home_contexts` gains a
**liveness column** — the max `updated_at` across the context's **visibility-scoped** resources
(scoped through `resources_visible_to` + `is_active`, mirroring the existing `resource_count`
subquery so recency and count agree on the counted set). `HomeContext` gains a corresponding
field (e.g. `last_active_at: Option<OffsetDateTime>` or a pre-derived recency score — decide the
wire shape against the harness so the glow curve is computable client-side).

**Migration: edit in place.** `graph_home_contexts` lives in the **branch-local, unshipped**
migration `20260707140000_graph_home_build_research_reads.sql`. Per the Beat B polish precedent
(and `[[feedback_shipped_migrations_immutable]]` applying only to *shipped* migrations), add the
recency column by **editing that migration in place + resetting the dev DB** — not a new
DROP+CREATE migration. (Were the branch about to PR before C lands, a new migration would be
required instead; it is not.)

**Regenerate** the relevant `.sqlx` cache after the SQL change (the read is `query_as` in
`atlas_home`, so no macro cache — but verify) and the `ts-rs` types for the extended
`HomeContext`. All reads stay **visibility-scoped** — the recency column must not leak an
`updated_at` from a resource the caller cannot see (`[[feedback_read_gate_must_match_full_canonical_visibility]]`).

## 5. What gets deleted (the bulk of C)

The retirement is narrower than "delete the panorama" first sounds — `TierPanorama` and the
`Territory`/region machinery **survive for the cogmap axis** (`cogmap_panorama` is regions-only
and still needs them). What dies is the **entire `?team` scope** — which orphans three surfaces,
not one: the Tier-0 team overview, the Tier-2 **team neighborhood**, and the team-scoped
**search accelerator**. (The last is re-homed in a later beat, not lost — see §9.) Region
Tier-1/2 (`territory_slice` / `cogmap_neighborhood`) stay reachable through the cogmap door.

**Backend (`temper-services` / SQL / handlers):**
- `graph_service.rs::territory_overview` (the team fn that appends both axes,
  `graph_service.rs:513-620`) — **delete**.
- `graph_context_territories`, `graph_region_territories`, `graph_orphan_salient_nodes`,
  `graph_territory_bridges` SQL — **stop referencing now, DROP later.** All four are
  team-`territory_overview`-only (**verified**: `cogmap_panorama` uses `graph_cogmap_territories`
  / `graph_cogmap_orphan_nodes` and no bridges), but they live in migrations **shipped to
  `main`** — immutable, and a `DROP FUNCTION` is **non-additive** (would break temper's
  auto-deploying `main` in the migrate-ahead-of-deploy window). So C deletes only the Rust caller
  (`territory_overview`); the fns become dead DB objects and are dropped in a **separate additive
  migration after C deploys** (§9).
- `team_service.rs::team_scope` (`:497`, produces `TeamScopeView`) + the
  `/api/teams/{id}/graph-scope` handler (`handlers/teams.rs:240`) — **delete**.
- The team territory-overview handler `/api/teams/{id}/graph/territories`, the team
  neighborhood `/api/teams/{id}/graph/slice`, and the team search `/api/teams/{id}/graph/search`
  handlers (`handlers/graph.rs` / `handlers/teams.rs`) — **delete**. (Team `unified_search`
  entry retires with the search accelerator; the shared `unified_search` service itself stays —
  it backs other search.)

**Wire types (`temper-core` → generated):**
- `TerritoryKind` — **kept unchanged** (all three variants). The backend stops *emitting*
  `Context` (team `territory_overview` is deleted; `cogmap_panorama` emits only `Region`), but
  Home's **client-side** field layout still builds `Territory` objects with `kind: 'context'` /
  `'cogmap'` (`homeLayout.ts`), and `TERRITORY_TINTS` is keyed by `TerritoryKind`. Removing a
  variant would break Home's typecheck — so leave the enum intact; the backend just never emits
  `Context` now.
- `graph_scope.rs`: `TeamZone`, `TeamScopeView`, `TeamRef` (team-scope only) — **delete**
  (**verified** consumers: `team_service`, `handlers/teams`, `crumbModel.ts`, `viewData.ts`,
  `graph-reads.ts`, `AtlasCrumb.svelte` — all retired/edited here).
- `Territory`, `TerritoryOverview`, `OrphanNode`, `Bridge`, `RegionMember`, `TerritorySlice` —
  **survive** (cogmap panorama + Tier 1/2).

**Loader / server reads (temper-ui):**
- The entire **`?team` scope branch** in `graph/[owner]/+page.server.ts:170-218` — **delete**.
- `graph-reads.ts`: `readTeamScope`/`teamScopePath`, `readTerritories`/`territoriesPath`,
  `readNeighborhood`/`neighborhoodSlicePath`, `readAtlasSearch`/`atlasSearchPath` — **delete**
  (all team-scoped). Keep `readCogmapNeighborhood`, `readCogmapPanorama`, `readRegionSlice`,
  `readAtlasHome`, `readTrail`, `readResourceRow`.

**Frontend (temper-ui):**
- `TierPanorama.svelte` — **survives for cogmaps**, sheds the zone-handling (`enterZone`,
  `TeamZoneMark`) and the inert-context branch (`TierPanorama.svelte:82-98`). Simplify to
  regions-only.
- `marks/TeamZoneMark.svelte` — **delete** (verify no cogmap use).
- `SearchAccelerator.svelte` + the `_search/+server.ts` endpoint's team path — **delete** (search
  is gated on `data.teamId` at `AtlasPage.svelte:63`; re-homed in a later beat, §9).
- `AtlasPage.svelte` — drop the `{#if data.teamId}` search block and the `teamId` plumbing;
  keep `cogmapId`.
- `crumbModel.ts` / `viewData.ts` — drop the `scope: TeamScopeView | null` field and its crumb
  derivation.
- `AtlasCrumb.svelte` — the team crumb (`:35`, `buildScopeUrl`) → a lightweight **`Home ›
  +tasker`** scope crumb reflecting `?scope`.
- `nav.ts` — `parseTeam` (`:32`), `buildScopeUrl` (team, `:149`) **retire**; **add** `?scope`
  builder + parser beside the `?home` builders.

**URL frame:** `?team` disappears from the Atlas entirely. Scope becomes a **Home filter**
(`?scope` alongside `?home`), not a tier.

## 6. URL / state frame (after C)

- **Home neutral:** `/graph/[owner]` (no params) — union haze.
- **Home committed lens:** `?home=build` / `?home=research`.
- **Home scoped:** `?home=build&scope=+tasker` (or `&scope=@me`, `&scope=temper` for research).
- **Cogmap panorama → territory → neighborhood:** `?cogmap=<id>` → `?focus=territory:X` →
  `?focus=…,node:Y` — **unchanged**.
- `?scope` set with **pushState** (Back returns to the un-narrowed lens), consistent with the
  `?home` drill-history convention. Reactive URL state must go through `goto`/`page.state`
  correctly — shallow `pushState` leaves `$page.url` stale
  (`[[reference_svelte_pushstate_leaves_page_url_stale]]`, the gotcha B hit).

## 7. Accessibility

- Scope chips are **real buttons/links** — keyboard-focusable, SR-labeled ("filter to +tasker"),
  Enter/Space to activate; the active chip carries `aria-pressed`/current.
- The Home a11y list fallback (`HomeA11yList`) gains a **scope-filter mirror** (the chip-row's
  non-spatial twin) and surfaces **recency as text metadata** per row ("last active …") beside
  the resource count.
- No field body becomes keyboard-dead under a scope narrow; clearing scope is always reachable.

## 8. Testing

- **`nav.ts`** — `?scope` build/parse round-trip; neutral = absent param; `?home` + `?scope`
  compose.
- **Home state machine** — extend B's reducer tests with the **commit → scope-narrow → clear**
  path (pure where possible): committing a lens exposes chips; narrowing sets `?scope` and filters
  the body set; clearing restores the union; Back pops scope before lens.
- **Client-side scope filter** — given an all-scopes build set, filtering by `owner_ref` yields
  exactly that scope's bodies; chip set derives from the present owner-scopes (build includes
  `@me`; research includes universal, excludes `@me`).
- **Recency encoding** — `TerritoryCircle` maps recency → glow independently of size (same input →
  same output, no `Math.random`).
- **Fixture guard** — extend the `home` scenario with the recency field; validate scope-chip
  derivation; `satisfies` the extended `HomeContext`; `sanitize-atlas-fixtures.mjs` updated; no
  personal-data leak in the committed synthetic bundle.
- **Backend e2e** — the recency column returns only **visibility-scoped** `updated_at` (a
  resource the caller can't see never advances a context's liveness) — a deny-direction test.
  Run the **access-sensitive e2e tier** (`[[feedback_access_semantics_changes_need_e2e_tier]]`),
  not just `test-db`; rebuild the spawned `temper` bin first if e2e execs the CLI
  (`[[feedback_nextest_does_not_rebuild_spawned_temper_bin]]`).
- **Deletion coverage** — the team-panorama e2e/unit tests that pin removed behavior
  (`tests/e2e/tests/graph_territory_overview_sql_test.rs`, `territory.test.ts`,
  `forceTerritories.test.ts` team cases, `graph-reads.paths.test.ts` team paths) are removed or
  retargeted — no orphaned tests referencing deleted reads.

## 9. Scope boundaries / captured for later

- **In scope (C):** retire the team panorama (backend fn + SQL + handler + wire types + loader
  branch + zone rendering); add the flat `?scope` filter on Home (client-side narrow + chips +
  a11y mirror + crumb); add the **recency** channel to the build read + field. Fixture + type
  regeneration.
- **Build-body destination stays put.** Clicking a context body still lands on
  `/vault/<owner>/<ctx>` (B §10.4's temp). **Beat E** (context-view re-imagining) owns
  redefining that destination — C does **not** touch it.
- **Chunk A descendant-enumeration stays dormant, not deleted.** It was the goal's riskiest,
  hardest-won access-navigation piece; keeping the unused backend semantics costs little and
  org-topology may want them later. Only the *zone-rendering UI* is deleted.
- **Research gets no recency.** Recency is the north star's contexts-view signal (§4.8); research
  sizing stays `region_count` (B §10.3 deferred research enrichment).
- **Re-home wayfinding search (its own follow-up beat).** Retiring `?team` orphans the
  search accelerator (gated on `data.teamId`, `AtlasPage.svelte:63`) — the only place Atlas
  search is wired. C **deletes** the team-scoped search wiring; a later beat re-homes wayfinding
  search at a post-reshape scope (footprint-wide via `resources_visible_to(profile)`, and/or
  per-cogmap), which needs a **net-new non-team search endpoint** and its own design. Folding it
  into C would balloon the beat; the north star's wayfinding surface (§5) returns in that beat.
  **Interim:** no search box in the Atlas between C landing and the re-home beat.
- **Drop the orphaned team SQL fns (follow-up additive migration).** `graph_region_territories`,
  `graph_context_territories`, `graph_orphan_salient_nodes`, `graph_territory_bridges` are left as
  dead (unreferenced) DB objects by C because they sit in shipped migrations and a `DROP` is
  non-additive ([[feedback_drop_function_non_additive_breaks_deploy_skew]]). Once C has deployed
  everywhere (no code references them), a small additive migration `DROP FUNCTION IF EXISTS`s all
  four. Sequencing safety, not new behavior.
- **Deferred:** org-topology / team-structure view (the retired downward zone-existence
  affordance, if ever wanted — admin/team-management surface, not the Atlas); per-cogmap
  resource/density sizing (B §10.3); context-view re-imagining (Beat E / Chunk D).

## 10. Decisions (locked with Cole; harness may refine the visuals in §3)

1. **Team panorama retired, not un-mixed (LOCKED).** Home's lenses + flat scope filter subsume
   it. No reach lost (§2.1).
2. **Scope filter is flat + client-side (LOCKED).** Distinct owner-scopes from the reachable set;
   the union is already fetched, so narrowing needs no backend read.
3. **Scope chips appear only after lens commit (LOCKED, harness may refine).** Not selectable at
   union-rest.
4. **Recency = a glow channel independent of size (LOCKED, harness may refine the curve).** Size =
   `resource_count`; glow = recency. Build only.
5. **Migration edited in place (LOCKED).** `graph_home_contexts` is branch-local + unshipped; add
   the recency column in `20260707140000` + reset the dev DB.
6. **Chunk A enumeration dormant, not deleted (LOCKED).**

## 11. Connections

- North star `019f39ca` (§3 surface map, §4.8 team→contexts size+recency, §7 deferred aggregate);
  goal `019f28a1`; Beat A spec `2026-07-06-atlas-beat-a-cogmap-knowledge-field-spec.md`; Beat B
  spec `2026-07-07-atlas-beat-b-home-reframe-spec.md` (§0 sequencing payoff, §10.4 build-body
  temp).
- `[[project_graph_atlas_visualization_goal]]` — this is the reshape arc's third beat.
- `[[feedback_local_proddata_render_harness_for_ui]]` — `/dev/atlas` locks the §3 visual
  specifics in the plan spike.
- `[[feedback_read_gate_must_match_full_canonical_visibility]]` — the recency column must honor
  the full visibility predicate (no unseen `updated_at` leak).
- `[[feedback_access_semantics_changes_need_e2e_tier]]` — recency read change runs the
  access-sensitive e2e tier.
- `[[reference_svelte_pushstate_leaves_page_url_stale]]` — `?scope` reactive-URL handling.
- `[[feedback_shipped_migrations_immutable]]` — edit-in-place is safe *because* the migration is
  unshipped.
- `[[reference_atlas_region_ids_ephemeral]]` — cogmap/context ids here are stable; the caution is
  about steward-materialized region ids (untouched by C).
