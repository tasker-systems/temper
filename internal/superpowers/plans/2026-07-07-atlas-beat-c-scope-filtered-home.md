# Atlas Beat C — retire team panorama; scope-filtered Home — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire the `?team` Atlas panorama entirely and let Beat B's Home lenses subsume it via a flat client-side `?scope` filter, adding a recency ("what's alive") glow channel to the build field.

**Architecture:** Beat B's build read is already access-complete (every reachable context, all scopes), so per-team narrowing is a pure client-side array filter — no new backend read. The only backend change is a **recency column** on `graph_home_contexts` (edited in place; the migration is branch-local + unshipped). The bulk of the beat is a **teardown**: the whole `?team` scope (Tier-0 team overview, Tier-2 team neighborhood, and the team-gated search accelerator) is deleted; regions stay reachable through the cogmap door, which has its own regions-only panorama/slice/neighborhood path. Search re-homing is a deferred follow-up beat.

**Tech Stack:** SvelteKit (Svelte 5 runes) + TypeScript (temper-ui); Rust (temper-services / temper-api / temper-core with `ts-rs`); Postgres + sqlx; Vitest (UI) + cargo-nextest (Rust) + e2e crate.

## Global Constraints

- **Branch:** `jct/atlas-reshape` (held — build, do not PR). Commit per beat-task locally.
- **Spec:** `docs/superpowers/specs/2026-07-07-atlas-beat-c-scope-filtered-home-spec.md` — every task's requirements implicitly include it.
- **Migration edited in place** (`20260707140000_...`) because it is branch-local + unshipped; after editing, `cargo make db-reset` (drop+recreate+re-apply) — never rely on an incremental `migrate run` for an edited file.
- **Fixture-first (`[[feedback_local_proddata_render_harness_for_ui]]`):** visual specifics (scope-chip treatment, recency→glow curve) are locked on `/dev/atlas` in Task 1, then implemented. Do not guess visuals ahead of the spike.
- **Visibility-scoped reads:** the recency column must never expose an `updated_at` from a resource the caller can't see — reuse the exact join set of the existing `resource_count` subquery (`resources_visible_to` + `is_active`).
- **Access-semantics changes run the e2e tier**, not just `test-db` (`[[feedback_access_semantics_changes_need_e2e_tier]]`); rebuild the spawned `temper` bin first if e2e execs the CLI (`[[feedback_nextest_does_not_rebuild_spawned_temper_bin]]`).
- **SQL macro cache:** the Home reads use runtime `query_as` (no macro cache), but after any SQL change run `cargo make check` (offline sqlx) to confirm. If a `.sqlx` regeneration is needed, `cargo make prepare-services`.
- **`cargo fmt` before every commit** (pre-commit gates on `fmt --check`; `[[feedback_implementer_subagents_must_run_fmt]]`).
- **Reactive URL:** `?scope` changes go through `goto` (not shallow `pushState`, which leaves `$page.url` stale — `[[reference_svelte_pushstate_leaves_page_url_stale]]`).

**Run from repo root** `/Users/petetaylor/projects/tasker-systems/temper` unless a step says otherwise.

---

### Task 1: `/dev/atlas` spike — lock the scope-chip + recency-glow visuals (fixture-first)

**Files:**
- Modify (gitignored, not committed): `packages/temper-ui/static/dev/atlas-fixtures.local.json`
- Modify: `packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs` (add the new `last_active_at` field to the `home` sanitizer if it strips unknown keys)
- Output: append locked decisions to the spec's §10 (a small doc edit) — no production `.svelte`/`.rs` committed from this task beyond the sanitizer.

**Interfaces:**
- Produces (decisions the later tasks consume): (a) scope-chip **placement + treatment**; (b) whether scope chips animate; (c) the **recency→glow curve** shape and its `now`-reference handling; (d) confirmation the recency wire shape is a **raw `last_active_at` timestamp** (client derives the curve) vs a server-normalized score. Default carried into later tasks: **raw `last_active_at: string | null`**, client-side pure `recencyGlow(lastActiveAt, now)`.

- [ ] **Step 1: Copy the committed `home` fixture into the local override and hand-shape it**

Copy `static/dev/atlas-fixtures.json`'s `home` scenario into `static/dev/atlas-fixtures.local.json`. Hand-edit the `build` array so it contains contexts across **at least three owner-scopes** (`@me`, `+tasker`, `+temper`) and add a `last_active_at` ISO-8601 string to each with a spread of ages (some hours old, some months old) so the glow channel is visible. Example body shape per build entry:

```json
{ "id": "…uuid…", "name": "tasker", "owner_ref": "+tasker", "resource_count": 42, "last_active_at": "2026-07-05T18:30:00Z" }
```

- [ ] **Step 2: Point the dev server at the override and open Home**

Run: `cd packages/temper-ui && bun run dev`
Open `http://localhost:5173/dev/atlas` (the harness bypasses the loader — `[[feedback_local_proddata_render_harness_for_ui]]`), select the `home` scenario, commit the **build** lens.
Expected: the build field renders bodies sized by `resource_count`, tinted by `owner_ref` (Beat B behavior).

- [ ] **Step 3: Prototype the scope chip-row and recency glow inline in `TierHome.svelte`**

Iterate directly on `TierHome.svelte` against the harness (this is exploratory — the committed implementation lands in Task 6). Try: a horizontal chip-row between the CTAs and the field, one chip per distinct `owner_ref` present (+ "All"), each in its `buildTint(owner_ref)` color; clicking narrows the rendered set. Separately, drive `TerritoryCircle` glow from `last_active_at` age instead of size. Tune until the two channels (size = magnitude, glow = liveness) read as distinct.

- [ ] **Step 4: Lock the decisions into the spec §10**

Append to the spec's §10 the locked specifics: chip placement/treatment, whether chips appear only after commit (default yes), the recency→glow curve (e.g. `glow = clamp(exp(-ageDays / HALFLIFE))`, with `HALFLIFE` value), and the confirmed wire shape (`last_active_at` raw timestamp). These become the contract Tasks 5–6 implement.

- [ ] **Step 5: Revert the exploratory `TierHome.svelte` edits; keep only the sanitizer + spec edits**

Run: `git checkout packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte`
Keep the `sanitize-atlas-fixtures.mjs` edit and the spec §10 edit. The `.local.json` is gitignored (never committed).

- [ ] **Step 6: Commit the spike output**

```bash
cargo fmt --manifest-path Cargo.toml
git add docs/superpowers/specs/2026-07-07-atlas-beat-c-scope-filtered-home-spec.md packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs
git commit -m "spike(atlas): Beat C Task 1 — lock scope-chip + recency-glow visuals on /dev/atlas"
```

---

### Task 2: Backend teardown — delete the team panorama reads, service, handlers, wire types

**Files:**
- Modify: `crates/temper-services/src/services/graph_service.rs` (delete `territory_overview`, `graph_service.rs:513-620`)
- Modify: `crates/temper-services/src/services/team_service.rs` (delete `team_scope`, `:497-556`, and its now-unused imports at `:20`)
- Modify: `crates/temper-api/src/handlers/graph.rs` (delete the `/api/teams/{id}/graph/territories` + `/api/teams/{id}/graph/slice` handlers) and `crates/temper-api/src/handlers/teams.rs` (delete the `/api/teams/{id}/graph-scope` + `/api/teams/{id}/graph/search` handlers, `:232-240`) + their route registrations
- Modify: `crates/temper-core/src/types/graph_scope.rs` (delete the whole file's types) + `crates/temper-core/src/types/mod.rs:76` (drop the `graph_scope` re-export + `mod graph_scope`)
- **Do NOT touch any migration.** The four team SQL fns live in migrations SHIPPED to `main` (`20260703130000`, `20260706120300`) — immutable ([[feedback_shipped_migrations_immutable]]), and a `DROP FUNCTION` is non-additive: it would break temper's auto-deploying `main` in the migrate-ahead-of-deploy window ([[feedback_drop_function_non_additive_breaks_deploy_skew]]). This task deletes only the **Rust callers** so nothing references them; the fns stay as dead DB objects and are dropped in a **separate follow-up additive migration after C deploys** (captured in the ledger + spec §9).
- Delete: `tests/e2e/tests/graph_territory_overview_sql_test.rs`
- Test: `cargo make check` + the temper-services/temper-api unit suites stay green

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: the team-scope backend surface is gone. `graph_service` still exports `cogmap_panorama`, `territory_slice`, `atlas_home` (unchanged). `TeamZone`/`TeamScopeView`/`TeamRef` no longer exist — Task 3 removes the frontend consumers (do Task 3 **first if** you want the build green at every step; see note).

> **Ordering note:** deleting the wire types here breaks the frontend generated-type consumers until Task 3. Because temper-core (Rust) and temper-ui (TS) build separately, the **Rust** workspace stays green after this task; the **TS** typecheck goes red until Task 3. If you require green-at-every-task including TS, swap Task 2 and Task 3 (do frontend teardown first). Recommended: **Task 3 before Task 2** so `bun run check` never goes red. This plan lists backend first for readability; execute FE teardown first.

- [ ] **Step 1: (resolved by controller) — the team SQL fns are SHIPPED; do not touch migrations**

The four fns are on `main` (confirmed by the controller). Leave all migrations untouched. The fns become dead (unreferenced) DB objects after this task and are dropped later in a separate additive migration. **No `cargo make db-reset`, no new migration in this task.**

- [ ] **Step 2: Delete `territory_overview` and `team_scope`**

Remove `graph_service.rs::territory_overview` (the whole fn, `:513-620`) and `team_service.rs::team_scope` (`:497-556`) plus any now-unused `use` (`TeamRef, TeamScopeView, TeamZone`, and the region/context/orphan/bridge query bindings). Leave `cogmap_panorama`, `territory_slice`, `atlas_home` untouched. (The SQL fns they called stay in the DB, now unreferenced — that's intentional.)

- [ ] **Step 3: (skipped — see Step 1) no SQL/migration change**

- [ ] **Step 4: Delete the handlers + routes + wire types**

Remove the four team-graph handlers and their `.route(...)` registrations (territories, slice, graph-scope, search) and the `utoipa` path attrs. Delete `crates/temper-core/src/types/graph_scope.rs`, remove `pub mod graph_scope;` + the `pub use graph_scope::...` from `crates/temper-core/src/types/mod.rs`. Delete `tests/e2e/tests/graph_territory_overview_sql_test.rs`.

- [ ] **Step 5: Regenerate ts-rs types (drops `graph_scope.ts`) and verify build**

Run: `cargo make generate-ts-types`
Then: `cargo make check`
Expected: Rust workspace compiles; `packages/temper-ui/src/lib/types/generated/graph_scope.ts` is gone. (TS typecheck may be red here — resolved by Task 3 / FE-first ordering.)

- [ ] **Step 6: Commit**

```bash
cargo fmt --manifest-path Cargo.toml
git add crates/ migrations/ tests/e2e/ packages/temper-ui/src/lib/types/generated/
git commit -m "refactor(atlas): Beat C teardown (backend) — delete team panorama reads/service/handlers/types"
```

---

### Task 3: Frontend teardown — delete `?team` scope, team reads, search accelerator

**Files:**
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` (delete the team branch `:170-218` and the now-unused imports `readTeamScope, readTerritories, readNeighborhood, parseTeam`)
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts` (delete `teamScopePath`, `readTeamScope`, `territoriesPath`, `readTerritories`, `neighborhoodSlicePath`, `readNeighborhood`, `atlasSearchPath`, `readAtlasSearch`, and the now-unused `TeamScopeView` import)
- Delete: `packages/temper-ui/src/lib/components/graph/atlas/SearchAccelerator.svelte`
- Modify: `packages/temper-ui/src/routes/(app)/graph/_search/+server.ts` (delete — it only served team search)
- Delete: `packages/temper-ui/src/lib/components/graph/atlas/marks/TeamZoneMark.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte` (remove `enterZone`, `TeamZoneMark` usage, zones prop, and the `kind === 'region'` inert-context branch — regions-only now)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/AtlasPage.svelte` (drop the `{#if data.teamId}<SearchAccelerator/>` block `:63-64`, and `teamId` plumbing `:56,78`)
- Modify: `packages/temper-ui/src/lib/graph/atlas/viewData.ts` (drop `scope: TeamScopeView | null` `:29` + the `TeamScopeView` import `:13`; keep `teamId`? — set it permanently null or remove; remove)
- Modify: `packages/temper-ui/src/lib/graph/atlas/crumbModel.ts` (drop the `scope` input field + the `input.scope` branch `:33-36`; the cogmap branch stays)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/AtlasCrumb.svelte` (drop the `scope`/team-crumb wiring; keep cogmap crumb)
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts` (delete `parseTeam` `:32`, `buildScopeUrl` `:149`; leave `buildCogmapUrl` — but drop its `p.delete('team')` line since `team` no longer exists)
- Test: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts` (drop the team-path cases), `packages/temper-ui/src/lib/graph/atlas/crumbModel.test.ts` (drop team-scope crumb cases)

**Interfaces:**
- Consumes: nothing.
- Produces: the loader has only two branches (`cogmap` and no-scope Home). `nav.ts` no longer exports `parseTeam`/`buildScopeUrl`. `viewData`/`crumbModel` no longer carry `scope`. Task 6 re-introduces a scope-filter crumb segment keyed on `?scope`, not `TeamScopeView`.

- [ ] **Step 1: Delete the team loader branch + unused imports**

In `+page.server.ts`, remove everything from `const filters = parseFilters(...)` / `const scope = await readTeamScope(...)` through the final team `return {...}` (`:170-218`), and drop `readTeamScope, readTerritories, readNeighborhood` and `parseTeam` from the imports. The `if (!teamId)` Home branch becomes the unconditional fall-through after the `cogmap` branch (drop the `if (!teamId)` guard — nothing sets team anymore).

- [ ] **Step 2: Delete the team reads from `graph-reads.ts`**

Remove the eight team-scoped exports listed above + the `TeamScopeView` import. Keep `readCogmapPanorama`, `readCogmapNeighborhood`, `cogmapNeighborhoodSlicePath`, `readRegionSlice`, `readAtlasHome`, `readTrail`, `readResourceRow`, `listTeams`.

- [ ] **Step 3: Delete SearchAccelerator + `_search` endpoint + TeamZoneMark**

```bash
git rm packages/temper-ui/src/lib/components/graph/atlas/SearchAccelerator.svelte \
       packages/temper-ui/src/routes/\(app\)/graph/_search/+server.ts \
       packages/temper-ui/src/lib/components/graph/atlas/marks/TeamZoneMark.svelte
```
Then remove the `SearchAccelerator` import + `{#if data.teamId}` block and `teamId` props from `AtlasPage.svelte`.

- [ ] **Step 4: Simplify TierPanorama to regions-only + strip scope from viewData/crumb/nav**

In `TierPanorama.svelte`: delete `enterZone`, the `zones` prop, `TeamZoneMark` render, and change the per-territory `onEnter`/`intensity` so every territory is a region (drop the `t.kind === 'region' ? … : undefined` guards — all are regions). In `viewData.ts` drop `scope` + import. In `crumbModel.ts` drop the `scope` field from `CrumbInput` and the `if (input.scope) {…}` block. In `AtlasCrumb.svelte` drop the scope prop. In `nav.ts` delete `parseTeam` + `buildScopeUrl`, and remove the `p.delete('team')` line inside `buildCogmapUrl`.

- [ ] **Step 5: Fix the path/crumb tests, then run the UI checks**

Edit `graph-reads.paths.test.ts` to drop the team-path assertions; edit `crumbModel.test.ts` to drop team-scope crumb cases. Then:
Run: `cd packages/temper-ui && bun run check && bun run test`
Expected: svelte-check clean (no dangling `TeamScopeView`/`readTeamScope`/`parseTeam` refs), all vitest green.

- [ ] **Step 6: Grep for dangling references, then commit**

Run: `rg -n "TeamScopeView|readTeamScope|readTerritories|readNeighborhood\b|parseTeam|buildScopeUrl|SearchAccelerator|TeamZoneMark|readAtlasSearch|\.get\('team'\)|searchParams.*team" packages/temper-ui/src`
Expected: no production hits (only comments/history if any).

```bash
cargo fmt --manifest-path Cargo.toml
git add packages/temper-ui
git commit -m "refactor(atlas): Beat C teardown (frontend) — delete ?team scope, team reads, search accelerator"
```

---

### Task 4: `nav.ts` — the `?scope` Home filter (pure, TDD)

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `parseScopeFilter(url: URL): string | null` — the `?scope` owner_ref, or null.
  - `buildScopeFilterUrl(base: URL, scope: string): string` — set `?scope` (call site PUSHes history).
  - `clearScopeFilterUrl(base: URL): string` — delete `?scope`.
  - Invariant: `?scope` only meaningful alongside a committed `?home` lens; the builders don't enforce it (the component gates rendering), they just set/clear the param.

- [ ] **Step 1: Write the failing test**

```ts
// nav.test.ts — add to the existing suite
import { describe, it, expect } from 'vitest';
import { parseScopeFilter, buildScopeFilterUrl, clearScopeFilterUrl } from './nav';

describe('scope filter (?scope)', () => {
	const u = (qs: string) => new URL(`https://x/graph/@me${qs}`);

	it('parses absent scope as null', () => {
		expect(parseScopeFilter(u(''))).toBeNull();
		expect(parseScopeFilter(u('?home=build'))).toBeNull();
	});

	it('parses a present scope', () => {
		expect(parseScopeFilter(u('?home=build&scope=%2Btasker'))).toBe('+tasker');
		expect(parseScopeFilter(u('?home=build&scope=%40me'))).toBe('@me');
	});

	it('builds a scope filter preserving the committed lens', () => {
		expect(buildScopeFilterUrl(u('?home=build'), '+tasker')).toBe('/graph/@me?home=build&scope=%2Btasker');
	});

	it('clears the scope filter, keeping the lens', () => {
		expect(clearScopeFilterUrl(u('?home=build&scope=%2Btasker'))).toBe('/graph/@me?home=build');
	});
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd packages/temper-ui && bun run test -- nav.test.ts`
Expected: FAIL — `parseScopeFilter is not a function`.

- [ ] **Step 3: Implement the three helpers**

```ts
// nav.ts — add beside the ?home helpers
/** The active Home scope filter (`?scope=@me|+slug|temper`), or null for the un-narrowed lens. */
export function parseScopeFilter(url: URL): string | null {
	return url.searchParams.get('scope');
}

/** Narrow the committed Home lens to one owner-scope (call site PUSHes history). */
export function buildScopeFilterUrl(base: URL, scope: string): string {
	return withParams(base, (p) => p.set('scope', scope));
}

/** Clear the scope narrow, returning to the full committed lens. */
export function clearScopeFilterUrl(base: URL): string {
	return withParams(base, (p) => p.delete('scope'));
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd packages/temper-ui && bun run test -- nav.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path Cargo.toml
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): Beat C — ?scope Home filter nav helpers"
```

---

### Task 5: Recency in the build read (`graph_home_contexts` + `HomeContext` + `atlas_home`)

**Files:**
- Modify: `migrations/20260707140000_graph_home_build_research_reads.sql` (edit in place — branch-local + unshipped)
- Modify: `crates/temper-core/src/types/graph_home.rs` (`HomeContext` gains `last_active_at`)
- Modify: `crates/temper-services/src/services/graph_service.rs::atlas_home` (select + map the new column)
- Test: `tests/e2e/tests/` — new `graph_home_recency_test.rs` (visibility-scoped recency, deny-direction)

**Interfaces:**
- Consumes: Task 1's locked wire shape (default: raw `last_active_at` timestamp).
- Produces: `HomeContext { id, name, owner_ref, resource_count, last_active_at: Option<OffsetDateTime> }`; generated TS `HomeContext.last_active_at: string | null`. Task 6 consumes `last_active_at`.

- [ ] **Step 1: Write the failing e2e test (visibility-scoped recency)**

```rust
// tests/e2e/tests/graph_home_recency_test.rs
// Uses the shared harness (mirror graph_territory_overview_sql_test's setup / home e2e).
// Asserts: (1) a context's last_active_at reflects the max updated_at of its VISIBLE,
// is_active resources; (2) a resource the caller cannot see does NOT advance it; (3) a
// context with no visible resources returns NULL last_active_at.
#[sqlx::test(migrator = "…the crate migrator…")]
async fn home_recency_is_visibility_scoped(pool: PgPool) {
	// seed: profile P; context C owned by P; resource R1 (visible, updated T1);
	// resource R2 (NOT visible to P, updated T2 > T1) homed in C.
	// call graph_home_contexts(P); find C.
	// assert last_active_at == T1 (R2's newer stamp must NOT leak).
	// (Fill in with the harness's seed helpers — home_resource_returning, etc.,
	//  from the Beat B polish e2e; add an updated_at setter if none exists.)
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e -- graph_home_recency`
Expected: FAIL — the column `last_active_at` doesn't exist yet (or the fn returns 4 columns).

- [ ] **Step 3: Add the recency column to `graph_home_contexts` (edit in place)**

In `migrations/20260707140000_...sql`, change the `RETURNS TABLE(...)` to add `last_active_at timestamptz`, and add a correlated subquery mirroring the existing `resource_count` join set exactly (so recency and count agree on the counted set):

```sql
CREATE FUNCTION graph_home_contexts(p_profile uuid)
RETURNS TABLE(context_id uuid, name text, owner_ref text, resource_count int, last_active_at timestamptz)
LANGUAGE sql STABLE AS $$
    -- … reachable_teams, candidates CTEs unchanged …
    SELECT c.id, c.name,
           CASE … END AS owner_ref,
           (SELECT count(*)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id)::int AS resource_count,
           (SELECT max(rr.updated_at)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            JOIN kb_resources rr ON rr.id = h.resource_id AND rr.is_active
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id) AS last_active_at
    FROM candidates cand
    JOIN kb_contexts c ON c.id = cand.id
    -- … LEFT JOINs unchanged …
    WHERE context_visible_to(p_profile, c.id)
    ORDER BY owner_ref, c.name;
$$;
```

Then reset the dev DB: `cargo make db-reset`.

- [ ] **Step 4: Extend `HomeContext` + `atlas_home`**

In `graph_home.rs`, add `pub last_active_at: Option<time::OffsetDateTime>,` to `HomeContext` (match the crate's existing time type + `ts_rs` mapping used elsewhere; if the crate uses `chrono`, use that). In `graph_service.rs::atlas_home`, change the build query tuple to 5 columns and the map:

```rust
let build: Vec<HomeContext> = sqlx::query_as::<_, (Uuid, String, String, i32, Option<OffsetDateTime>)>(
    "SELECT context_id, name, owner_ref, resource_count, last_active_at FROM graph_home_contexts($1)",
)
.bind(profile_id.as_uuid())
.fetch_all(pool)
.await?
.into_iter()
.map(|(id, name, owner_ref, resource_count, last_active_at)| HomeContext {
    id, name, owner_ref, resource_count, last_active_at,
})
.collect();
```

- [ ] **Step 5: Regenerate TS types + run the e2e test**

Run: `cargo make generate-ts-types` (adds `last_active_at: string | null` to generated `graph_home.ts`)
Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e -- graph_home_recency`
Expected: PASS (recency reflects only visible resources; unseen newer resource does not leak).
Then: `cargo make check` (confirm offline sqlx + clippy clean).

- [ ] **Step 6: Commit**

```bash
cargo fmt --manifest-path Cargo.toml
git add migrations/ crates/ tests/e2e/ packages/temper-ui/src/lib/types/generated/
git commit -m "feat(atlas): Beat C — visibility-scoped recency (last_active_at) on graph_home_contexts"
```

---

### Task 6: Home scope-filter interaction — chips, client-side filter, recency glow, crumb

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/homeTint.ts` (add pure `recencyGlow`)
- Modify: `packages/temper-ui/src/lib/graph/atlas/scopeChips.ts` (**create** — pure chip derivation)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte` (chip-row, `?scope` client filter, recency glow wiring, commit→scope→clear)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte` (accept an optional `glow` channel independent of `intensity`/size — per Task 1's locked shape)
- Modify: `packages/temper-ui/src/lib/graph/atlas/crumbModel.ts` (add a `scopeFilter: string | null` input → a `Home › +scope` segment) + `viewData.ts` (carry `scopeFilter`) + `AtlasCrumb.svelte`
- Test: `packages/temper-ui/src/lib/graph/atlas/scopeChips.test.ts`, `homeTint.test.ts` (recencyGlow cases), `crumbModel.test.ts` (scope segment)

**Interfaces:**
- Consumes: `parseScopeFilter`/`buildScopeFilterUrl`/`clearScopeFilterUrl` (Task 4); `HomeContext.last_active_at` (Task 5); Task 1's locked glow curve.
- Produces:
  - `deriveScopeChips(bodies: { owner_ref: string }[]): string[]` — distinct owner_refs in stable order (the chip set; "All" is implicit in the component).
  - `recencyGlow(lastActiveAt: string | null, now: number): number` — pure `[0,1]` liveness, `now` injected for determinism.

- [ ] **Step 1: Write failing tests for the pure helpers**

```ts
// scopeChips.test.ts
import { describe, it, expect } from 'vitest';
import { deriveScopeChips } from './scopeChips';
describe('deriveScopeChips', () => {
	it('returns distinct owner_refs in stable (sorted) order', () => {
		expect(deriveScopeChips([{ owner_ref: '+tasker' }, { owner_ref: '@me' }, { owner_ref: '+tasker' }]))
			.toEqual(['+tasker', '@me']); // sorted: '+' < '@' by charCode
	});
	it('is empty for no bodies', () => { expect(deriveScopeChips([])).toEqual([]); });
});

// homeTint.test.ts — add
import { recencyGlow } from './homeTint';
describe('recencyGlow', () => {
	const now = Date.parse('2026-07-07T00:00:00Z');
	it('is null-safe → 0 glow for never-active', () => { expect(recencyGlow(null, now)).toBe(0); });
	it('is ~max for just-active', () => { expect(recencyGlow('2026-07-07T00:00:00Z', now)).toBeGreaterThan(0.9); });
	it('decays for old activity', () => {
		expect(recencyGlow('2026-01-01T00:00:00Z', now)).toBeLessThan(recencyGlow('2026-07-01T00:00:00Z', now));
	});
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd packages/temper-ui && bun run test -- scopeChips.test.ts homeTint.test.ts`
Expected: FAIL — `deriveScopeChips` / `recencyGlow` not defined.

- [ ] **Step 3: Implement the pure helpers**

```ts
// scopeChips.ts
/** The distinct owner-scopes present in a lens's bodies, sorted for a stable chip order. */
export function deriveScopeChips(bodies: { owner_ref: string }[]): string[] {
	return [...new Set(bodies.map((b) => b.owner_ref))].sort();
}
```

```ts
// homeTint.ts — add (curve constant from Task 1's spike; HALFLIFE_DAYS default 14)
const RECENCY_HALFLIFE_DAYS = 14;
/** Liveness glow [0,1] from last-active age; `now` (ms) injected for deterministic tests. */
export function recencyGlow(lastActiveAt: string | null, now: number): number {
	if (!lastActiveAt) return 0;
	const ageDays = Math.max(0, (now - Date.parse(lastActiveAt)) / 86_400_000);
	return Math.min(1, Math.exp(-ageDays / RECENCY_HALFLIFE_DAYS));
}
```

- [ ] **Step 4: Run to verify they pass**

Run: `cd packages/temper-ui && bun run test -- scopeChips.test.ts homeTint.test.ts`
Expected: PASS.

- [ ] **Step 5: Wire the chip-row + client filter + glow into `TierHome.svelte`**

Add, per Task 1's locked layout: a chip-row (shown only when `committed`), one chip per `deriveScopeChips` entry (colored `buildTint(ref)` / `researchTint(ref)`) plus an "All" chip; clicking a chip calls `goto(buildScopeFilterUrl($page.url, ref), {...})`, "All"/active calls `clearScopeFilterUrl`. Derive `const scope = $derived(parseScopeFilter($page.url))`; filter the rendered `buildPos`/`researchPos` bodies to `scope == null || ownerRefById.get(t.id) === scope`. Pass `glow={recencyGlow(lastActiveById.get(t.id) ?? null, Date.now())}` to build `TerritoryCircle`s (research passes no glow). Keep size (`r`) driven by `member_count` as today. Add `lastActiveById` beside `ownerRefById`.

- [ ] **Step 6: Add the independent `glow` channel to `TerritoryCircle.svelte`**

Add an optional `glow?: number` prop (default `undefined`). When present, it drives the field-effect glow/opacity **independently** of `intensity` (which stays size-linked), per Task 1's locked treatment. When absent, behavior is unchanged (research + panorama unaffected).

- [ ] **Step 7: Add the scope crumb segment**

In `crumbModel.ts`, add `scopeFilter: string | null` to `CrumbInput`; when set (and no cogmap), push a `{ label: scopeFilter, kind: 'scope', focusPath: null }` segment after home. Add `'scope'` to the `kind` union. Thread `scopeFilter` through `viewData.ts` (from `parseScopeFilter` in the loader's Home branch) and `AtlasCrumb.svelte`. Update `crumbModel.test.ts` with a scope-segment case.

- [ ] **Step 8: Run the UI suite + typecheck**

Run: `cd packages/temper-ui && bun run check && bun run test`
Expected: svelte-check clean; all vitest green.

- [ ] **Step 9: Commit**

```bash
cargo fmt --manifest-path Cargo.toml
git add packages/temper-ui
git commit -m "feat(atlas): Beat C — Home scope-filter chips + recency glow + scope crumb"
```

---

### Task 7: A11y mirror + fixture guard

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/HomeA11yList.svelte` (scope-filter mirror + recency text)
- Modify: `packages/temper-ui/src/lib/graph/atlas/fixtures.ts` / the committed `static/dev/atlas-fixtures.json` `home` scenario (add `last_active_at` to build entries)
- Modify: `packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs` (already touched in Task 1 — confirm `last_active_at` preserved)
- Test: `packages/temper-ui/src/lib/graph/atlas/fixtures.test.ts` (home scenario carries recency; key-set pinned via `satisfies HomeContext`)

**Interfaces:**
- Consumes: Task 5's `last_active_at`, Task 6's `deriveScopeChips`.
- Produces: the accessible twin of the chip-row + recency; the fixture guard pins the new shape.

- [ ] **Step 1: Write the failing fixture-guard test**

```ts
// fixtures.test.ts — extend the home-scenario assertions
it('home build entries carry a recency field and no leaked PII', () => {
	const home = loadFixture('home'); // existing helper
	for (const c of home.build) {
		expect(c).toHaveProperty('last_active_at'); // string | null
		// key-set pinned to the wire type:
		const _pin = c satisfies import('$lib/types/generated/graph_home').HomeContext;
	}
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd packages/temper-ui && bun run test -- fixtures.test.ts`
Expected: FAIL — committed `home` fixture build entries lack `last_active_at`.

- [ ] **Step 3: Add `last_active_at` to the committed home fixture + confirm sanitizer**

Add a synthetic `last_active_at` ISO string to each `home.build` entry in the committed `static/dev/atlas-fixtures.json`. Confirm `sanitize-atlas-fixtures.mjs` carries the field (no PII — timestamps are synthetic). Re-run Step 2's test → PASS.

- [ ] **Step 4: Add the scope-filter mirror + recency text to `HomeA11yList.svelte`**

Render the scope chips as a real `<button>` group (SR label "filter to +tasker", `aria-pressed` on the active one; Enter/Space activate via the same `buildScopeFilterUrl`/`clearScopeFilterUrl` gotos), and add "last active …" text per build row beside its resource count (from `last_active_at`; "—" when null). The list respects the active `?scope` (filtered set matches the field).

- [ ] **Step 5: Run the full UI suite + a11y-adjacent checks**

Run: `cd packages/temper-ui && bun run check && bun run test`
Expected: svelte-check clean; all vitest green.

- [ ] **Step 6: Commit**

```bash
cargo fmt --manifest-path Cargo.toml
git add packages/temper-ui
git commit -m "feat(atlas): Beat C — a11y scope mirror + recency text + fixture guard"
```

---

## Self-Review

**Spec coverage:**
- §2 retire team panorama → Tasks 2 + 3. ✓
- §2.2 flat scope filter (client-side) → Tasks 4 + 6. ✓
- §3 scope-chip interaction (after commit, tint-band chips, derived-from-bodies) → Tasks 1 (lock) + 6. ✓
- §3 recency glow channel → Tasks 1 (lock curve) + 5 (read) + 6 (wire). ✓
- §4 recency enrichment, migration edit-in-place, visibility-scoped → Task 5. ✓
- §5 deletions (service/SQL/handlers/wire types/loader/reads/SearchAccelerator/TeamZoneMark/crumb/nav) → Tasks 2 + 3. ✓
- §6 URL frame (`?team` gone, `?scope` added) → Tasks 3 (remove) + 4 (add). ✓
- §7 a11y (chips as buttons, list mirror, recency text) → Tasks 6 + 7. ✓
- §8 testing (nav round-trip, state machine, client filter, recency encoding, fixture guard, backend e2e deny-direction, deletion coverage) → Tasks 4/5/6/7 + deletions in 2/3. ✓
- §9 search re-homing deferred (delete wiring now) → Task 3. ✓
- §10 decisions locked → carried as Task constraints; Task 1 appends visual specifics. ✓

**Placeholder scan:** the only deliberately-deferred value is Task 1's glow-curve tuning, which carries a concrete default (`RECENCY_HALFLIFE_DAYS = 14`, `exp` decay) so Tasks 5–6 are implementable without the spike; the spike may adjust the constant. The e2e seed helpers in Task 5 Step 1 reference the existing Beat B home-e2e harness helpers (`home_resource_returning`, `soft_delete_resource`) rather than inventing them. No `TBD`/`TODO` left.

**Type consistency:** `HomeContext.last_active_at: Option<OffsetDateTime>` (Rust) ↔ `last_active_at: string | null` (TS) ↔ fixture string ↔ `recencyGlow(lastActiveAt: string | null, now: number)`. `deriveScopeChips` consumes `{ owner_ref }` matching `HomeContext`. `parseScopeFilter`/`buildScopeFilterUrl`/`clearScopeFilterUrl` names consistent across Tasks 4/6/7. `TerritoryKind` unchanged (spec §5 corrected). ✓

**Ordering caveat surfaced:** execute **Task 3 (FE teardown) before Task 2 (BE teardown)** to keep `bun run check` green throughout (Task 2 deletes the wire types the FE consumes). Noted in Task 2's ordering note.
