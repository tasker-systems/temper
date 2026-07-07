# Atlas Beat B — Home Reframe (build / research field) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the placeholder `you → teams → cogmaps` Atlas Home with a JTBD **build / research** verb-lens field — two CTAs over one Beat-A field panel; hover resolves a lens, click commits, Back returns to neutral — never surfacing "context"/"cogmap" jargon.

**Architecture:** Frontend-led, fixture-first. A hand-shaped `/dev/atlas` `home` fixture drives the read contract; the UI reuses Beat A's `forceTerritories` + `TerritoryCircle` field-effect. Pure logic (URL lens state, the rest→hover→commit reducer, the per-lens field layout) is built test-first as standalone modules, then `TierHome` consumes them. The backend Home read is reshaped **after** the fixture locks the shape: a new `graph_home_contexts` SQL function (personal + member-team contexts, visibility-scoped, sized by resource count) feeds the `build` list; the existing cogmap read feeds `research`.

**Tech Stack:** SvelteKit 5 (runes) + TypeScript + d3-force (frontend); Rust (temper-core wire types, temper-services SQL, temper-api handler) + sqlx + PostgreSQL; ts-rs codegen; Vitest (unit), cargo-nextest + `test-db`/e2e (backend).

## Global Constraints

- **No ontology on the surface.** The strings "context" and "cognitive map"/"cogmap" never appear in Home UI copy. Verbs **build** / **research**, never nouns "builder"/"researcher".
- **Reuse, don't reinvent** the Beat A field language: `forceTerritories` (deterministic, no `Math.random`), `TerritoryCircle` (intensity → glow+opacity, size → magnitude), the top-K label gate. `TerritoryKind` already includes `"context"` and `"cogmap"` with tints in `palette.ts`.
- **Visibility-scoped reads only.** Every context/cogmap in the Home read is gated by `resources_visible_to` / `contexts_visible_to` / `cogmap_visible_maps`. A context/cogmap the caller cannot see never appears. Include a **deny-direction** e2e test.
- **URI frame.** The committed lens lives in the URL (`?home=build|research`); neutral = no param. Commit uses **pushState** (Back returns to neutral); hover-preview is ephemeral (no URL write). Consistent with `nav.ts`.
- **Determinism.** All layout is pure and deterministic — same input → same positions, no `Math.random` / `Date.now()`.
- **SDD execution split** (`[[feedback_sdd_subagents_stall_on_backgrounded_cargo]]`): implementer subagents write code + tests; the **controller** runs every `cargo`/DB/`vitest`/`bun run check` and all commits. Controller runs `cargo fmt` before any Rust commit (`[[feedback_implementer_subagents_must_run_fmt]]`).
- **Additive/coordinated wire change.** `AtlasHome` is reshaped (teams/cogmaps → build/research). Read + UI ship together in one PR; the branch is held (not merging piecemeal), so no prod deploy-skew window. No shipped-migration edits — new migration file only (`[[feedback_shipped_migrations_immutable]]`).
- **Branch:** continue on `jct/atlas-reshape` (held). Commit per-beat locally; do not push/PR without asking (`[[feedback_commit_per_beat_authorized]]`, `[[feedback_always_push_pr_never_merge_local]]`).

---

## File Structure

**Frontend (`packages/temper-ui/src`)**
- `lib/graph/atlas/nav.ts` — *modify*: add `?home` lens builder + parser.
- `lib/graph/atlas/homeLens.ts` — *create*: pure rest→hover→commit reducer + types.
- `lib/graph/atlas/layout/homeLayout.ts` — *rewrite*: three-column `layoutHome` → per-lens field layout over `forceTerritories`.
- `lib/graph/atlas/palette.ts` — *modify*: `BUILD_LENS`/`RESEARCH_LENS` tints (one source of truth).
- `lib/components/graph/atlas/TierHome.svelte` — *rewrite*: two CTAs + field panel + state machine.
- `lib/components/graph/atlas/marks/*` — *reuse* `TerritoryCircle.svelte` (no change expected).
- `lib/types/generated/graph_home.ts` — *regenerated* from Rust.
- `routes/(app)/graph/[owner]/+page.server.ts` — *modify*: no-scope Home branch consumes `{build, research}` + threads `?home`.
- `lib/components/graph/atlas/AtlasPage.svelte` — *modify*: pass new Home props.
- `static/dev/atlas-fixtures.json` + `scripts/sanitize-atlas-fixtures.mjs` + `lib/graph/atlas/fixtures.test.ts` — *modify*: new `home` shape.

**Backend**
- `crates/temper-core/src/types/graph_home.rs` — *modify*: `HomeContext`; `AtlasHome { build, research }`.
- `migrations/2026070714xxxx_graph_home_contexts.sql` — *create*: `graph_home_contexts(p_profile)`.
- `crates/temper-services/src/services/graph_service.rs` — *modify*: `atlas_home` returns build/research.
- `crates/temper-api/src/handlers/graph.rs` — *touch*: handler already returns `AtlasHome` (type-driven; verify OpenAPI body).
- `crates/temper-api/src/lib.rs` — *touch* after adding the migration (compile-time migrator embed).
- `tests/e2e/tests/…` — *modify/create*: Home read visibility + deny-direction.

---

## Task 1: Harness spike — lock the fixture shape + interaction feel (INTERACTIVE)

> **Not a subagent task, not TDD.** This is the fixture-first design-discovery loop (`[[feedback_local_proddata_render_harness_for_ui]]`): the controller drives `/dev/atlas` with the human in the loop. Its deliverable is a **locked data shape** and an **agreed interaction**, which unblock the TDD tasks. Exit criteria are explicit; no production tests here.

**Files:**
- Create (gitignored): `packages/temper-ui/static/dev/atlas-fixtures.local.json` (copy of committed bundle, hand-shaped `home`).
- Scratch-modify: `lib/components/graph/atlas/TierHome.svelte` (rough prototype — hardened in Task 7).

- [ ] **Step 1: Copy the committed fixture to the local override**

```bash
cd packages/temper-ui
cp static/dev/atlas-fixtures.json static/dev/atlas-fixtures.local.json
```

- [ ] **Step 2: Hand-shape the `home` scenario to the target contract**

In `atlas-fixtures.local.json`, replace the `home` entry's `{ teams, cogmaps }` with the provisional Beat-B shape (spec §4). Give ~6–10 build contexts (personal `@me/*` + a couple `+team/*`) with varied `resource_count`, and ~4–8 research cogmaps with varied `region_count`:

```jsonc
"home": {
  "build": [
    { "id": "<uuid>", "name": "temper",     "owner_ref": "@me",        "resource_count": 331 },
    { "id": "<uuid>", "name": "storyteller", "owner_ref": "@me",        "resource_count": 42 },
    { "id": "<uuid>", "name": "roadmap",     "owner_ref": "+acme",      "resource_count": 18 }
    /* … */
  ],
  "research": [
    { "id": "<uuid>", "name": "Temper self-cognition", "region_count": 12 },
    { "id": "<uuid>", "name": "Acme platform",         "region_count": 5 }
    /* … */
  ]
}
```

- [ ] **Step 3: Prototype the interaction in `TierHome.svelte` against the fixture**

Rough-in (no tests yet) the target interaction so it can be felt on the harness:
- two verb-CTAs `build` / `research` + taglines;
- rest = hazy undifferentiated field (Beat A glow/haze, unresolved);
- hover a CTA → that lens's bodies resolve crisp via `forceTerritories` + `TerritoryCircle` (build = `context` tint, research = `cogmap` tint), the other dims;
- click → commit to that lens only; Back → neutral.

Map each `build` context → `Territory { kind: 'context', member_count: resource_count, … }` and each `research` cogmap → `Territory { kind: 'cogmap', member_count: region_count, … }` so `forceTerritories` sizes them (it weights `context`/`cogmap` by `member_count`).

- [ ] **Step 4: Iterate on the harness with the human until locked**

```bash
bun run dev   # → http://localhost:5173/dev/atlas , scenario: home
```

Drive the presets; confirm with the human:
- **§10.1 rest-haze semantics** — ambient atmosphere vs hazy union of both lenses.
- **§10.2 per-scope tint** — one build color vs subtle personal/team tint.
- Hover-resolve / click-commit / Back feel; sizing legibility at `short 1280×380` and `tall 1440×900`.

- [ ] **Step 5: Record the locked decisions**

Update spec §10.1/§10.2 in `docs/superpowers/specs/2026-07-07-atlas-beat-b-home-reframe-spec.md` with the chosen answers. Confirm the final field names in the `home` fixture (they are the contract for Tasks 2–9). **Exit criteria:** (a) human-approved interaction; (b) frozen `home` fixture shape.

- [ ] **Step 6: Commit the spec update (fixture override stays gitignored)**

```bash
git add docs/superpowers/specs/2026-07-07-atlas-beat-b-home-reframe-spec.md
git commit -m "docs(atlas): Beat B — lock Home rest-haze + tint decisions from harness spike"
```

---

## Task 2: Wire type — `AtlasHome { build, research }`

**Files:**
- Modify: `crates/temper-core/src/types/graph_home.rs`
- Regenerate: `packages/temper-ui/src/lib/types/generated/graph_home.ts`

**Interfaces:**
- Produces (Rust): `HomeContext { id: Uuid, name: String, owner_ref: String, resource_count: i32 }`; `AtlasHome { build: Vec<HomeContext>, research: Vec<HomeCogmap> }`. `HomeCogmap` keeps `{ id, name, team_ids, region_count, facet_count }`.
- Produces (TS, generated): `HomeContext`, `AtlasHome = { build: HomeContext[], research: HomeCogmap[] }`.

- [ ] **Step 1: Replace `HomeTeam` with `HomeContext` and reshape `AtlasHome`**

In `graph_home.rs`, remove `HomeTeam`, add `HomeContext`, reshape `AtlasHome` (keep the derive/cfg_attr stack from the existing structs verbatim):

```rust
/// A context the profile can build in — personal (`@me`) or team — sized by its
/// visible resource count. `owner_ref` is the decorated scope (`@me`, `+team-slug`).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeContext {
    pub id: Uuid,
    pub name: String,
    pub owner_ref: String,
    pub resource_count: i32,
}

/// The Atlas Home footprint, lensed by act: `build` = your contexts, `research`
/// = the cogmaps you can reach. Drops the `you` node (self implied).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasHome {
    pub build: Vec<HomeContext>,
    pub research: Vec<HomeCogmap>,
}
```

Update the module doc comment (drop "you→teams→cogmaps").

- [ ] **Step 2: Controller — compile-check core + regenerate TS types**

```bash
cargo check -p temper-core --all-features
cargo make generate-ts-types
```
Expected: compiles; `graph_home.ts` now exports `HomeContext` + reshaped `AtlasHome`. (temper-services / temper-ui will not compile yet — later tasks fix the consumers. That's expected mid-plan.)

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/types/graph_home.rs packages/temper-ui/src/lib/types/generated/graph_home.ts
git commit -m "feat(atlas): Beat B Home wire type — AtlasHome { build contexts, research cogmaps }"
```

---

## Task 3: `nav.ts` — `?home` lens URL state

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Produces: `type HomeLens = 'build' | 'research'`; `parseHomeLens(url: URL): HomeLens | null`; `buildHomeLensUrl(base: URL, lens: HomeLens): string`; `clearHomeLensUrl(base: URL): string`.

- [ ] **Step 1: Write the failing tests**

Add to `nav.test.ts`:

```ts
import { parseHomeLens, buildHomeLensUrl, clearHomeLensUrl } from './nav';

const u = (s: string) => new URL(`https://x.test/graph/@me${s}`);

test('parseHomeLens: absent → null, valid → value, garbage → null', () => {
	expect(parseHomeLens(u(''))).toBeNull();
	expect(parseHomeLens(u('?home=build'))).toBe('build');
	expect(parseHomeLens(u('?home=research'))).toBe('research');
	expect(parseHomeLens(u('?home=nope'))).toBeNull();
});

test('buildHomeLensUrl sets ?home and preserves path; clear removes it', () => {
	expect(buildHomeLensUrl(u(''), 'build')).toBe('/graph/@me?home=build');
	expect(clearHomeLensUrl(u('?home=research'))).toBe('/graph/@me');
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/nav.test.ts`
Expected: FAIL — `parseHomeLens` is not exported.

- [ ] **Step 3: Implement in `nav.ts`**

```ts
export type HomeLens = 'build' | 'research';

/** The committed Home lens, or null for the neutral (rest) state. */
export function parseHomeLens(url: URL): HomeLens | null {
	const v = url.searchParams.get('home');
	return v === 'build' || v === 'research' ? v : null;
}

/** Commit a Home lens (call site PUSHes history so Back returns to neutral). */
export function buildHomeLensUrl(base: URL, lens: HomeLens): string {
	return withParams(base, (p) => p.set('home', lens));
}

/** Return to the neutral Home selection. */
export function clearHomeLensUrl(base: URL): string {
	return withParams(base, (p) => p.delete('home'));
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/nav.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): Beat B ?home lens URL builder + parser"
```

---

## Task 4: `homeLens.ts` — the rest → hover → commit reducer

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/homeLens.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/homeLens.test.ts`

**Interfaces:**
- Produces: `type HomeLens = 'build' | 'research'` (re-exported from nav); `interface HomeLensState { committed: HomeLens | null; hover: HomeLens | null }`; `resolvedLens(s): HomeLens | null` (committed wins, else hover); `otherDimmed(s): boolean` (a field is dimmed only while previewing with nothing committed); pure transitions `hoverLens`, `clearHover`, `commitLens`, `clearCommit`.

- [ ] **Step 1: Write the failing tests**

```ts
import { resolvedLens, otherDimmed, hoverLens, clearHover, commitLens, clearCommit, type HomeLensState } from './homeLens';

const rest: HomeLensState = { committed: null, hover: null };

test('rest: nothing resolved, nothing dimmed', () => {
	expect(resolvedLens(rest)).toBeNull();
	expect(otherDimmed(rest)).toBe(false);
});

test('hover previews a lens and dims the other; commit removes the dim', () => {
	const h = hoverLens(rest, 'build');
	expect(resolvedLens(h)).toBe('build');
	expect(otherDimmed(h)).toBe(true);            // preview: other hazes behind
	const c = commitLens(h, 'build');
	expect(resolvedLens(c)).toBe('build');
	expect(otherDimmed(c)).toBe(false);           // committed: other is gone, not dimmed
});

test('committed lens ignores hover of the other for resolution; clearCommit → neutral', () => {
	const c = commitLens(rest, 'research');
	expect(resolvedLens(hoverLens(c, 'build'))).toBe('research'); // commit wins
	expect(resolvedLens(clearCommit(c))).toBeNull();
});

test('clearHover returns to prior committed/neutral', () => {
	expect(clearHover(hoverLens(rest, 'build'))).toEqual(rest);
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/homeLens.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `homeLens.ts`**

```ts
import type { HomeLens } from './nav';
export type { HomeLens } from './nav';

/** Home lens machine: a committed lens (URL-backed) and an ephemeral hover preview. */
export interface HomeLensState {
	committed: HomeLens | null;
	hover: HomeLens | null;
}

/** Committed wins; else the hover preview; else neutral. */
export function resolvedLens(s: HomeLensState): HomeLens | null {
	return s.committed ?? s.hover;
}

/** The non-resolved field is dimmed ONLY while previewing with nothing committed;
 *  once committed, the other field is not shown at all (spec §3). */
export function otherDimmed(s: HomeLensState): boolean {
	return s.committed === null && s.hover !== null;
}

export function hoverLens(s: HomeLensState, lens: HomeLens): HomeLensState {
	return { ...s, hover: lens };
}
export function clearHover(s: HomeLensState): HomeLensState {
	return { ...s, hover: null };
}
export function commitLens(s: HomeLensState, lens: HomeLens): HomeLensState {
	return { committed: lens, hover: null };
}
export function clearCommit(s: HomeLensState): HomeLensState {
	return { committed: null, hover: null };
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/homeLens.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/homeLens.ts packages/temper-ui/src/lib/graph/atlas/homeLens.test.ts
git commit -m "feat(atlas): Beat B Home lens reducer — rest/hover-preview/commit"
```

---

## Task 5: `homeLayout.ts` — per-lens field layout (reuse `forceTerritories`) + lens tints

**Files:**
- Rewrite: `packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.ts`
- Rewrite: `packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.test.ts`
- Modify: `packages/temper-ui/src/lib/graph/atlas/palette.ts` (lens tints)

**Interfaces:**
- Consumes: `AtlasHome`, `HomeContext`, `HomeCogmap` (generated TS, Task 2); `forceTerritories(territories, size)` → `PositionedTerritory[]`; `Territory` (`{ id, kind, label, member_count, salience, anchor_id }`).
- Produces: `buildLensTerritories(home: AtlasHome): Territory[]` (contexts → `kind:'context'`, `member_count: resource_count`); `researchLensTerritories(home: AtlasHome): Territory[]` (cogmaps → `kind:'cogmap'`, `member_count: region_count`); `layoutHomeLens(territories: Territory[], size): PositionedTerritory[]` (thin pass-through to `forceTerritories`, kept for a named seam + test point). `palette.ts`: `BUILD_LENS`, `RESEARCH_LENS` tint constants.

- [ ] **Step 1: Write the failing tests**

Replace `homeLayout.test.ts` contents:

```ts
import { buildLensTerritories, researchLensTerritories, layoutHomeLens } from './homeLayout';
import type { AtlasHome } from '$lib/types/generated/graph_home';

const home: AtlasHome = {
	build: [
		{ id: 'c1', name: 'temper', owner_ref: '@me', resource_count: 331 },
		{ id: 'c2', name: 'storyteller', owner_ref: '@me', resource_count: 42 }
	],
	research: [{ id: 'm1', name: 'Self-cognition', team_ids: [], region_count: 12, facet_count: 3 }]
};

test('build lens maps contexts to context-kind territories sized by resource_count', () => {
	const ts = buildLensTerritories(home);
	expect(ts.map((t) => t.kind)).toEqual(['context', 'context']);
	expect(ts[0]).toMatchObject({ id: 'c1', label: 'temper', member_count: 331, anchor_id: 'c1' });
});

test('research lens maps cogmaps to cogmap-kind territories sized by region_count', () => {
	const ts = researchLensTerritories(home);
	expect(ts.map((t) => t.kind)).toEqual(['cogmap']);
	expect(ts[0]).toMatchObject({ id: 'm1', label: 'Self-cognition', member_count: 12, anchor_id: 'm1' });
});

test('layoutHomeLens is deterministic (same input → same positions)', () => {
	const ts = buildLensTerritories(home);
	const a = layoutHomeLens(ts, { width: 1280, height: 560 });
	const b = layoutHomeLens(ts, { width: 1280, height: 560 });
	expect(a).toEqual(b);
	expect(a.every((p) => Number.isFinite(p.x) && Number.isFinite(p.y))).toBe(true);
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/layout/homeLayout.test.ts`
Expected: FAIL — new exports missing (old `layoutHome` gone).

- [ ] **Step 3: Implement `homeLayout.ts`**

```ts
/**
 * Home field layout (Beat B): map each lens's members to `Territory`s and lay them
 * out with the shared, deterministic `forceTerritories`. Build = your contexts
 * (sized by resource count); research = reachable cogmaps (sized by region count).
 */
import type { AtlasHome } from '$lib/types/generated/graph_home';
import type { Territory } from '$lib/types/generated/graph_territory';
import { forceTerritories } from './forceTerritories';
import type { PositionedTerritory } from './packTerritories';

export function buildLensTerritories(home: AtlasHome): Territory[] {
	return home.build.map((c) => ({
		id: c.id,
		kind: 'context',
		label: c.name,
		member_count: c.resource_count,
		salience: null,
		anchor_id: c.id
	}));
}

export function researchLensTerritories(home: AtlasHome): Territory[] {
	return home.research.map((m) => ({
		id: m.id,
		kind: 'cogmap',
		label: m.name,
		member_count: m.region_count,
		salience: null,
		anchor_id: m.id
	}));
}

/** Named seam over `forceTerritories` (one lens at a time). Deterministic. */
export function layoutHomeLens(
	territories: Territory[],
	size: { width: number; height: number }
): PositionedTerritory[] {
	return forceTerritories(territories, size);
}
```

> If Task 2's generated `Territory` requires more fields than shown, mirror the exact optional/required set in the mapper — do not invent fields. Confirm via `graph_territory.ts`.

- [ ] **Step 4: Add lens tints to `palette.ts`**

```ts
/** Beat B Home lens tints: build reuses the context hue, research the cogmap hue,
 *  so Home agrees with the panorama wash it leads into. */
export const BUILD_LENS = { tint: TERRITORY_TINTS.context, ink: '#cfe0f6' } as const;
export const RESEARCH_LENS = { tint: TERRITORY_TINTS.cogmap, ink: '#f4d3a6' } as const;
```

- [ ] **Step 5: Run to verify pass + typecheck**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/layout/homeLayout.test.ts`
Expected: PASS. (`bun run check` will still fail until `TierHome` is rewritten — Task 7.)

- [ ] **Step 6: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.ts packages/temper-ui/src/lib/graph/atlas/layout/homeLayout.test.ts packages/temper-ui/src/lib/graph/atlas/palette.ts
git commit -m "feat(atlas): Beat B Home field layout — build/research lens territories + tints"
```

---

## Task 6: Backend — `graph_home_contexts` SQL + `atlas_home` returns build/research

**Files:**
- Create: `migrations/2026070714xxxx_graph_home_contexts.sql`
- Modify: `crates/temper-services/src/services/graph_service.rs` (`atlas_home`)
- Touch: `crates/temper-api/src/lib.rs` (after new migration; compile-time migrator embed — `[[feedback_shipped_migrations_immutable]]` context: only touch, never edit shipped migrations)
- Test: `tests/e2e/tests/…` (Home read visibility + deny-direction)

**Interfaces:**
- Consumes: `HomeContext`, `AtlasHome` (Task 2); existing `graph_home_cogmaps(p_profile)`.
- Produces: SQL `graph_home_contexts(p_profile uuid) RETURNS TABLE(context_id uuid, name text, owner_ref text, resource_count int)`; reshaped `atlas_home(pool, profile_id) -> ApiResult<AtlasHome>` returning `{ build, research }`.

- [ ] **Step 1: Confirm the context schema predicates before writing SQL**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
grep -rn "owner_table\|owner_id" migrations/*.sql | grep -i "kb_contexts" | head
grep -rn "CREATE TABLE kb_contexts\|CREATE TABLE kb_team_contexts" migrations/*.sql
```
Confirm: how a **personal** context is owned (`kb_contexts.owner_table = 'profiles'` + `owner_id = p_profile`, or equivalent), how a **team** context joins (`kb_team_contexts(team_id, context_id)` + member gate), and the per-context resource-count source (`kb_resource_homes` where `anchor_table='kb_contexts'`, gated by `resources_visible_to`). Mirror `resources_in_team_scope` (migration `20260703000002`).

- [ ] **Step 2: Write the migration `graph_home_contexts`**

New file `migrations/2026070714xxxx_graph_home_contexts.sql` (renumber to sort after the latest main migration at execution time). Draft — adjust the two owner predicates to the values confirmed in Step 1:

```sql
-- Home build lens: the profile's contexts — personal (owned by the profile) and
-- member-team contexts — each with its visible resource count. Visibility-scoped
-- per-context via resources_visible_to (a private resource never inflates a count
-- for a caller who can't see it). owner_ref is the decorated scope for the UI.
CREATE FUNCTION graph_home_contexts(p_profile uuid)
RETURNS TABLE(context_id uuid, name text, owner_ref text, resource_count int)
LANGUAGE sql STABLE AS $$
    WITH mine AS (
        -- personal contexts owned by the profile
        SELECT c.id, c.name, '@me'::text AS owner_ref
        FROM kb_contexts c
        WHERE c.owner_table = 'profiles' AND c.owner_id = p_profile AND c.is_active
        UNION
        -- contexts of teams the profile is a member of
        SELECT c.id, c.name, ('+' || t.slug)::text AS owner_ref
        FROM kb_team_contexts tc
        JOIN kb_contexts c ON c.id = tc.context_id AND c.is_active
        JOIN kb_teams t ON t.id = tc.team_id AND t.is_active
        JOIN kb_team_members tm ON tm.team_id = tc.team_id AND tm.profile_id = p_profile
    )
    SELECT m.id, m.name, m.owner_ref,
           (SELECT count(*)
            FROM kb_resource_homes h
            JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
            WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = m.id)::int
    FROM mine m
    ORDER BY m.name;
$$;
```

> If personal contexts and team contexts can duplicate (a personal context shared to a team), the `UNION` dedups by identical row; if `owner_ref` differs, prefer the personal row — refine only if Step 1 shows overlap is possible.

- [ ] **Step 3: Reshape `atlas_home` service**

In `graph_service.rs`, replace the `teams` query with a `build` query on `graph_home_contexts`; keep the cogmap query as `research`. Runtime `query_as` → **no `.sqlx` regen**.

```rust
pub async fn atlas_home(pool: &PgPool, profile_id: ProfileId) -> ApiResult<AtlasHome> {
    let build: Vec<HomeContext> = sqlx::query_as::<_, (Uuid, String, String, i32)>(
        "SELECT context_id, name, owner_ref, resource_count FROM graph_home_contexts($1)",
    )
    .bind(profile_id.as_uuid())
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, name, owner_ref, resource_count)| HomeContext { id, name, owner_ref, resource_count })
    .collect();

    let research: Vec<HomeCogmap> = sqlx::query_as::<_, (Uuid, String, Vec<Uuid>, i32, i32)>(
        "SELECT cogmap_id, name, team_ids, region_count, facet_count FROM graph_home_cogmaps($1)",
    )
    .bind(profile_id.as_uuid())
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, name, team_ids, region_count, facet_count)| HomeCogmap {
        id, name, team_ids, region_count, facet_count,
    })
    .collect();

    Ok(AtlasHome { build, research })
}
```
Update the `use` line: `graph_home::{AtlasHome, HomeCogmap, HomeContext}` (drop `HomeTeam`).

- [ ] **Step 4: Controller — apply migration, touch migrator, compile**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo make docker-up          # if not already up
sqlx migrate run
touch crates/temper-api/src/lib.rs      # re-embed the compile-time migrator
cargo check -p temper-services -p temper-api --all-features
```
Expected: compiles; migration applied.

- [ ] **Step 5: Write the e2e visibility + deny-direction test**

In the e2e suite (mirror an existing `atlas_home`/graph home test; if none, add `tests/e2e/tests/atlas_home_test.rs`), assert: a profile's Home `build` lists exactly its personal + member-team contexts with correct counts; `research` lists only visible cogmaps; and a **deny-direction** case — a context/cogmap the profile is not a member of / cannot see is **absent**, and a private resource invisible to the caller does **not** inflate a context's `resource_count` (`[[feedback_read_gate_must_match_full_canonical_visibility]]`).

- [ ] **Step 6: Controller — run the access-sensitive e2e tier**

```bash
cargo build -p temper-cli --bin temper      # if e2e spawns the CLI ([[feedback_nextest_does_not_rebuild_spawned_temper_bin]])
cargo make test-e2e
```
Expected: PASS, including the deny-direction assertions. (`[[feedback_access_semantics_changes_need_e2e_tier]]` — test-db alone is a false green here.)

- [ ] **Step 7: Controller — fmt + commit**

```bash
cargo fmt
git add migrations/2026070714xxxx_graph_home_contexts.sql crates/temper-services/src/services/graph_service.rs crates/temper-api/src/lib.rs tests/e2e/
git commit -m "feat(atlas): Beat B Home read — graph_home_contexts (build) + build/research atlas_home"
```

---

## Task 7: `TierHome.svelte` rebuild + page/AtlasPage wiring

**Files:**
- Rewrite: `packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte`
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/AtlasPage.svelte`

**Interfaces:**
- Consumes: `AtlasHome` (Task 2); `buildLensTerritories`/`researchLensTerritories`/`layoutHomeLens` (Task 5); `HomeLensState`/`resolvedLens`/`otherDimmed`/`hoverLens`/`clearHover`/`commitLens` (Task 4); `parseHomeLens`/`buildHomeLensUrl`/`clearHomeLensUrl` (Task 3); `TerritoryCircle`, `BUILD_LENS`/`RESEARCH_LENS`.
- Produces: a `TierHome` taking `{ home: AtlasHome; lens: HomeLens | null; width; height }` and rendering the two-CTA field with hover/commit/Back.

- [ ] **Step 1: Rewrite `TierHome.svelte`** (harden the Task 1 prototype against the tested modules)

- Props: `home: AtlasHome`, `lens: HomeLens | null` (from `?home`), `width`, `height`.
- Local `HomeLensState` seeded `{ committed: lens, hover: null }`; hover of a CTA → `hoverLens`, pointerleave → `clearHover`; click → `goto(buildHomeLensUrl($page.url, lens))` (pushState) and `commitLens`; a Back/neutral affordance → `goto(clearHomeLensUrl($page.url))`.
- Render CTAs `build` / `research` + taglines (copy from spec §2). Field: when `resolvedLens` is `build`, lay out `buildLensTerritories(home)` via `layoutHomeLens`; when `research`, the research set. Rest (`resolvedLens === null`) = hazy undifferentiated field per the Task-1 locked decision (§10.1). `otherDimmed` → render the non-resolved lens at low opacity behind (preview only).
- Each body: `TerritoryCircle` with `kind` (`context`/`cogmap`), `intensity` from magnitude, tinted per lens; `onEnter` navigates — build → `/vault/<owner_ref>/<ctx>` (spec §10.4 temporary destination), research → `buildCogmapUrl($page.url, id)`.
- Drop the `you` node, the three-column headers, and `layoutHome` usage. Keep the pointer-move-threshold click/pan guard from the current file (it's still needed on the bodies).

- [ ] **Step 2: Thread the read + lens through `+page.server.ts`**

In the no-scope Home branch: `readAtlasHome` now returns `{ build, research }`; return `home` (the whole `AtlasHome`) and `homeLens: parseHomeLens(url)`; drop the separate `teams`/`cogmaps` return keys.

- [ ] **Step 3: Update `AtlasPage.svelte`**

Where it renders `TierHome` at the home tier, pass `home={data.home}` and `lens={data.homeLens}` instead of `teams`/`cogmaps`. Update the cache-key expressions on lines ~17/43 if they referenced `teams`/`cogmaps` (they key on `'home'` — verify no break).

- [ ] **Step 4: Controller — typecheck + full UI check**

```bash
cd packages/temper-ui && bun run check
```
Expected: PASS (svelte-check + tsc + biome). Fix any `Territory` field mismatches by matching the generated type exactly.

- [ ] **Step 5: Controller — eyeball on the harness**

```bash
bun run dev   # /dev/atlas , scenario: home — confirm rest/hover/commit/Back match the Task-1 lock
```

- [ ] **Step 6: Commit**

```bash
git add packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte "packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts" packages/temper-ui/src/lib/components/graph/atlas/AtlasPage.svelte
git commit -m "feat(atlas): Beat B TierHome — build/research verb-lens field + ?home wiring"
```

---

## Task 8: Accessibility — per-lens list fallback

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte`
- Test: `packages/temper-ui/src/lib/graph/atlas/homeLayout.test.ts` (or a small `homeA11y.test.ts` if a helper is extracted)

**Interfaces:**
- Consumes: `AtlasHome`, the lens territory mappers (Task 5).

- [ ] **Step 1: Failing test for the a11y list model (if a helper is extracted)**

If list rows are derived by a pure helper, test it; else this task is component-level and verified via the check below. Example helper test:

```ts
import { lensListRows } from './homeLayout';
test('lensListRows: build rows carry name + owner_ref + resources', () => {
	const rows = lensListRows({ build: [{ id: 'c1', name: 'temper', owner_ref: '@me', resource_count: 331 }], research: [] }, 'build');
	expect(rows[0]).toMatchObject({ label: 'temper', meta: '@me · 331 resources', href: '/vault/@me/temper' });
});
```

- [ ] **Step 2 (if helper): implement `lensListRows`; else skip to Step 3**

- [ ] **Step 3: Add the list fallback to `TierHome`**

- Focus of a CTA resolves its field (focus = hover): wire `onfocus`/`onblur` to `hoverLens`/`clearHover`.
- Provide a visually-available-or-SR list per resolved lens: `<ul>` of links (context → vault href + `owner_ref · N resources`; cogmap → panorama + `N regions`), reusing Beat A's a11y-list pattern. Every body is keyboard-focusable + Enter-enterable (`atlas-focusable` role/tabindex, as Beat A).

- [ ] **Step 4: Controller — check + commit**

```bash
cd packages/temper-ui && bun run test && bun run check
git add packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte packages/temper-ui/src/lib/graph/atlas/
git commit -m "feat(atlas): Beat B Home a11y — focus-resolves-lens + per-lens list fallback"
```

---

## Task 9: Fixtures + sanitizer + guard test

**Files:**
- Modify: `packages/temper-ui/static/dev/atlas-fixtures.json` (committed synthetic)
- Modify: `packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs`
- Modify: `packages/temper-ui/src/lib/graph/atlas/fixtures.test.ts`

**Interfaces:**
- Consumes: the frozen `home` shape (Task 1); `AtlasHome` (Task 2).

- [ ] **Step 1: Update the fixture guard to the new `AtlasHome` shape**

In `fixtures.test.ts`, pin the `home` scenario key-set to `{ build, research }` via `satisfies Record<keyof AtlasHome, true>` (mirrors the existing per-scenario guard), and assert no personal-data leak in `build[].name`/`owner_ref` / `research[].name`.

- [ ] **Step 2: Run to verify failure**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/fixtures.test.ts`
Expected: FAIL — committed `home` still has `{ teams, cogmaps }`.

- [ ] **Step 3: Regenerate the committed synthetic bundle**

Update `sanitize-atlas-fixtures.mjs` to remap the new `home` fields (build `owner_ref`/`name`, research `name`), then regenerate from the local capture:

```bash
cd packages/temper-ui
node scripts/sanitize-atlas-fixtures.mjs   # → static/dev/atlas-fixtures.json
```
If no fresh personal capture exists, hand-edit the committed `home` to the new shape with synthetic values (the sanitizer test guards cleanliness).

- [ ] **Step 4: Run to verify pass**

Run: `cd packages/temper-ui && bun run test src/lib/graph/atlas/fixtures.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/static/dev/atlas-fixtures.json packages/temper-ui/scripts/sanitize-atlas-fixtures.mjs packages/temper-ui/src/lib/graph/atlas/fixtures.test.ts
git commit -m "test(atlas): Beat B — home fixture reshaped to build/research + guard"
```

---

## Task 10: Full-gate green + consolidated review prep

- [ ] **Step 1: Controller — full workspace gate**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo make check                 # fmt + clippy + docs + machete (offline sqlx — honest local probe)
cd packages/temper-ui && bun run check && bun run test
cd /Users/petetaylor/projects/tasker-systems/temper && cargo make test-e2e
```
Expected: all green. (`cargo make check` is the honest sqlx-offline probe; the runtime `query_as` needs no cache, but confirm no macro query slipped in.)

- [ ] **Step 2: Consolidated review** (`[[feedback_subagent_review_cadence]]`)

Defer all spec + code-quality review to a single consolidated pass here (opus review across the beat), rather than per-task. Address Critical/Important inline; log Minors as deferred follow-ups.

- [ ] **Step 3: Prod-verify note**

On eventual PR, prod-verify the Home field/hover on temperkb.io (auth is prod-only — `[[reference_vercel_preview_no_auth0_verify_in_prod]]`). Branch stays **held** until then.

---

## Self-Review (against the spec)

- **§1–2 reframe / verbs-not-nouns / no-ontology-leak** → Global Constraints + Task 7 copy (spec §2). ✓
- **§3 interaction (rest/hover/commit/Back)** → Task 4 reducer + Task 7 component + Task 3 URL. ✓
- **§3 build lens = contexts sized by resources; research = cogmaps sized by regions** → Task 5 mappers + Task 6 read. ✓
- **§4 fixture-first contract** → Task 1 spike locks shape; Tasks 6/9 implement + guard. ✓
- **§5 `?home` URL state, pushState Back** → Task 3 + Task 7 Step 1. ✓
- **§6 a11y (focus=hover, list fallback)** → Task 8. ✓
- **§7 frontend files** → Tasks 3/4/5/7/8. **§8 backend** → Tasks 2/6. **§9 testing** → per-task + Task 10. ✓
- **§10 open decisions** → Task 1 Steps 4–5 (haze, tint); §10.3 defer (research=region_count, Task 5); §10.4 temporary vault destination (Task 7 Step 1). ✓
- **§11 C-thread captured** → spec only; no code (correct). ✓
- **Type consistency:** `HomeContext { id, name, owner_ref, resource_count }` used identically in Tasks 2/5/6/8/9; `HomeLensState`/`resolvedLens`/`otherDimmed` consistent Tasks 4/7; `?home` param name consistent Tasks 3/7. ✓
