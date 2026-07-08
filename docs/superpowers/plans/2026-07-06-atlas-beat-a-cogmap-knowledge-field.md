# Atlas Beat A — Cogmap Knowledge Field Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the cogmap panorama into an orientation-grade knowledge *field*: force-separated regions sized + glow/opacity-weighted by salience, labels gated to the salient few (mixed-case, below-circle), a resources·salience·coherence hover, an a11y list fallback, and the missing derived-label + coherence read.

**Architecture:** Backend adds a derived label (Problem 2) and `content_cohesion` to the cogmap territory read; `Territory` gains `coherence`. Frontend swaps the tight circle-pack for a deterministic force layout and a salience field-effect, with pure helpers (`wrapLabel`, `fieldStyle`, `labeledRegionIds`, `intensityOf`) extracted so logic is unit-tested and the `.svelte` marks are verified on the `/dev/atlas` harness.

**Tech Stack:** Rust (sqlx, axum, temper-services/temper-core), SvelteKit + TypeScript, d3-force, Vitest, cargo-nextest, ts-rs.

**Spec:** `docs/superpowers/specs/2026-07-06-atlas-beat-a-cogmap-knowledge-field-spec.md` (vault `019f39e2`). **North star:** `…2026-07-06-atlas-reshape-projection-class-north-star.md`. **Subsumes** task `019f38b3`.

## Global Constraints

- **Branch:** `jct/atlas-reshape` (already checked out; the two docs are committed on it). Do NOT commit to `main` (auto-deploys).
- **Cargo-stall split** (`feedback_sdd_subagents_stall_on_backgrounded_cargo`): a subagent WRITES code + tests; the **controller** runs `sqlx migrate run`, cargo/nextest, `cargo make check`, `cargo make generate-ts-types`, and all commits. Subagents never background cargo.
- **Review cadence** (`feedback_subagent_review_cadence`): defer spec + code review to ONE consolidated pass at the end of the plan, not per task.
- **Migrations are immutable once applied**; additive-only on `main`. The one migration here is a new file (`feedback_shipped_migrations_immutable`).
- **`--all-features`** for Rust builds/clippy. Rust uses cargo-nextest. TS uses Vitest + Biome.
- **fmt before commit** (`feedback_implementer_subagents_must_run_fmt`): controller runs `cargo make fix`/`cargo fmt` before `cargo make check` (pre-commit gates on fmt).
- **DB env for bare cargo:** `export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.
- Exact tuned values (intensity formulas, K, easing) are copied from the spec §2/§4 verbatim below — do not re-derive.

---

## File Structure

**Backend**
- Create `migrations/2026070613xxxx_cogmap_territory_derived_label_and_coherence.sql` — DROP+CREATE `graph_cogmap_territories` (derive + coherence).
- Modify `crates/temper-core/src/types/graph_territory.rs` — `Territory.coherence: Option<f64>`.
- Modify `crates/temper-services/src/services/graph_service.rs` — `cogmap_panorama` reads the 6th column; `territory_overview` sets `coherence: None`.

**Frontend (pure helpers — unit-tested)**
- Modify `packages/temper-ui/src/lib/graph/atlas/labels.ts` — add `wrapLabel`, `fieldStyle`, `labeledRegionIds`, `intensityOf`.
- Create `packages/temper-ui/src/lib/graph/atlas/labels.test.ts` (or extend) — tests for the four helpers.
- Create `packages/temper-ui/src/lib/graph/atlas/layout/forceTerritories.ts` (+ `.test.ts`) — deterministic force layout.

**Frontend (marks — harness-verified)**
- Modify `marks/TerritoryCircle.svelte` — intensity field-effect, below-circle wrapped label, `<title>`.
- Modify `TierPanorama.svelte` — force layout, gating, intensity, sparse-cogmap mixed-case, a11y list.
- Create `marks/RegionHoverCard.svelte` — resources·salience·coherence card.

**Fixtures / tests**
- Modify `packages/temper-ui/static/dev/atlas-fixtures.json` + `scripts/sanitize-atlas-fixtures.mjs` — enrich with labels + coherence.
- Add/extend an e2e test for the cogmap panorama read.

---

## Task 1: Backend — derived label + coherence on the cogmap territory read

**Files:**
- Create: `migrations/2026070613xxxx_cogmap_territory_derived_label_and_coherence.sql`
- Modify: `crates/temper-core/src/types/graph_territory.rs:28-38` (Territory struct)
- Modify: `crates/temper-services/src/services/graph_service.rs` (`cogmap_panorama` ~657-676; `territory_overview` ~548-557)
- Test: `tests/e2e/tests/graph_atlas_test.rs` (or the existing atlas e2e file — grep first)

**Interfaces:**
- Produces: `Territory.coherence: Option<f64>`; `graph_cogmap_territories(uuid,uuid,uuid) → (region_id, cogmap_id, label, member_count, salience, coherence)`.

- [ ] **Step 1: Write the failing e2e test** — cogmap panorama returns a derived label for an unlabeled region and a coherence value, and never surfaces a non-visible member's title.

```rust
// tests/e2e/tests/<atlas file>.rs — add:
// Given a cogmap with a region whose stored label is NULL and one VISIBLE member
// titled "Alpha Concept", the panorama's territory label == "Alpha Concept" and
// coherence is Some(_). Given a region whose only member is NOT visible to the
// caller, its label is None (never the hidden title).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_panorama_derives_label_and_returns_coherence(pool: PgPool) {
    let ctx = seed_cogmap_with_regions(&pool).await; // helper: one labeled-NULL region + visible member; one private-member region
    let ov = cogmap_panorama(&pool, ctx.profile, ctx.cogmap, None).await.unwrap();
    let derived = ov.territories.iter().find(|t| t.id == ctx.region_visible).unwrap();
    assert_eq!(derived.label.as_deref(), Some("Alpha Concept"));
    assert!(derived.coherence.is_some());
    let hidden = ov.territories.iter().find(|t| t.id == ctx.region_private).unwrap();
    assert_eq!(hidden.label, None, "a non-visible member's title must never leak as a label");
}
```

- [ ] **Step 2: Controller runs it to confirm RED** — `export DATABASE_URL=…; cargo nextest run -p <e2e-crate> --features test-db --test <file> cogmap_panorama_derives`. Expected: FAIL (coherence field missing / label NULL).

- [ ] **Step 3: Add `coherence` to `Territory`**

```rust
// crates/temper-core/src/types/graph_territory.rs — inside `pub struct Territory`
    pub salience: Option<f64>,
    /// Region content cohesion (`content_cohesion`: mean member-to-centroid cosine).
    /// Sizes nothing — surfaced in the region hover card. None for contexts/cogmaps.
    pub coherence: Option<f64>,
    pub anchor_id: Uuid,
```

- [ ] **Step 4: Write the migration**

```sql
-- migrations/2026070613xxxx_cogmap_territory_derived_label_and_coherence.sql
-- Beat A: the cogmap panorama read gains B1's derived label (unlabeled region →
-- top VISIBLE member title, resources_visible_to discipline) AND returns
-- content_cohesion for the hover. Adding an OUT column changes the return type →
-- DROP + CREATE (CREATE OR REPLACE is illegal). Skew-safe: the sole caller selects
-- columns by name, so pre-deploy code selecting the old 5 keeps working.
DROP FUNCTION IF EXISTS graph_cogmap_territories(uuid, uuid, uuid);
CREATE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(region_id uuid, cogmap_id uuid, label text,
              member_count int, salience double precision, coherence double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, rep.title) AS label,
           reg.member_count, reg.salience, reg.content_cohesion
    FROM kb_cogmap_regions reg
    LEFT JOIN LATERAL (
        SELECT r.title
        FROM kb_cogmap_region_members m
        JOIN resources_visible_to(p_profile) v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) rep ON true
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;
```

- [ ] **Step 5: Update `cogmap_panorama` to read + map the 6th column**

```rust
// crates/temper-services/src/services/graph_service.rs — in cogmap_panorama
let territories: Vec<Territory> =
    sqlx::query_as::<_, (Uuid, Uuid, Option<String>, i32, f64, Option<f64>)>(
        "SELECT region_id, cogmap_id, label, member_count, salience, coherence \
             FROM graph_cogmap_territories($1, $2, $3)",
    )
    .bind(profile_id.as_uuid()).bind(cogmap_id).bind(lens)
    .fetch_all(pool).await?
    .into_iter()
    .map(|(region_id, cogmap_id, label, member_count, salience, coherence)| Territory {
        id: region_id,
        kind: TerritoryKind::Region,
        label,
        member_count,
        salience: Some(salience),
        coherence,
        anchor_id: cogmap_id,
    })
    .collect();
```

- [ ] **Step 6: Set `coherence: None` at every other `Territory { … }` construction** (`territory_overview` region + context builders, and any test/fixture builders). Grep `Territory {` across the workspace; add `coherence: None` where absent so it compiles.

- [ ] **Step 7: Controller applies migration + regenerates types + runs the test GREEN**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
sqlx migrate run
touch crates/temper-api/src/lib.rs   # force MIGRATOR embed rebuild (feedback_nextest_does_not_rebuild_spawned_temper_bin sibling)
cargo make generate-ts-types          # Territory.coherence → graph_territory.ts
cargo nextest run -p <e2e-crate> --features test-db --test <file> cogmap_panorama_derives
```
Expected: PASS. Commit (controller): migration + graph_territory.rs + graph_service.rs + regenerated `graph_territory.ts` + test.

---

## Task 2: `wrapLabel` helper + tests

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/labels.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/labels.test.ts`

**Interfaces:**
- Produces: `wrapLabel(text: string, cap: number, maxLines = 2): string[]`.

- [ ] **Step 1: Write failing tests**

```ts
import { describe, it, expect } from 'vitest';
import { wrapLabel } from './labels';
describe('wrapLabel', () => {
  it('keeps a short label on one line', () => expect(wrapLabel('Geology', 12)).toEqual(['Geology']));
  it('wraps a long label to two lines', () => expect(wrapLabel('The gap register', 8)).toEqual(['The gap', 'register']));
  it('ellipsis-truncates the final line when it overflows', () => {
    const r = wrapLabel('Narrative gravity as a runtime-recomputed field', 10);
    expect(r.length).toBe(2);
    expect(r[1].endsWith('…')).toBe(true);
  });
  it('truncates a single over-long word to one line', () => expect(wrapLabel('N-dimensional', 8)).toEqual(['N-dimen…']));
});
```

- [ ] **Step 2: Run RED** — `cd packages/temper-ui && bunx vitest run src/lib/graph/atlas/labels.test.ts`. Expected: FAIL (`wrapLabel` not exported).

- [ ] **Step 3: Implement `wrapLabel`** (append to `labels.ts`)

```ts
/** Greedy word-wrap into ≤ maxLines lines of ≤ cap chars; final line ellipsis-truncated. */
export function wrapLabel(text: string, cap: number, maxLines = 2): string[] {
	if (text.length <= cap) return [text];
	const words = text.split(/\s+/).filter(Boolean);
	const lines: string[] = [];
	let cur = '';
	for (let i = 0; i < words.length; i++) {
		const cand = cur ? `${cur} ${words[i]}` : words[i];
		if (cand.length <= cap || !cur) {
			cur = cand;
		} else {
			lines.push(cur);
			cur = words[i];
		}
		if (lines.length === maxLines - 1) {
			const rest = [cur, ...words.slice(i + 1)].join(' ');
			lines.push(truncateLabel(rest, cap));
			return lines;
		}
	}
	if (cur) lines.push(truncateLabel(cur, cap));
	return lines;
}
```

- [ ] **Step 4: Run GREEN** — same vitest command. Expected: PASS.
- [ ] **Step 5: Controller commits** — `git add labels.ts labels.test.ts && git commit -m "feat(atlas): wrapLabel two-line word-wrap helper"`.

---

## Task 3: `fieldStyle`, `labeledRegionIds`, `intensityOf` helpers + tests

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/labels.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/labels.test.ts`

**Interfaces:**
- Produces:
  - `intensityOf(salience: number | null, maxSalience: number): number` — `(s/max)^1.4`, clamped 0..1.
  - `fieldStyle(intensity: number, ghost: boolean): { fillOpacity: number; strokeOpacity: number; glowPx: number }`.
  - `labeledRegionIds(regions: {id: string; salience: number | null}[], k: number): Set<string>` — top-K by salience.

- [ ] **Step 1: Write failing tests**

```ts
import { intensityOf, fieldStyle, labeledRegionIds } from './labels';
describe('intensityOf', () => {
  it('maps max salience to 1 and eases the tail down', () => {
    expect(intensityOf(1, 1)).toBeCloseTo(1);
    expect(intensityOf(0.5, 1)).toBeLessThan(0.5); // exponent 1.4 pushes mid below linear
    expect(intensityOf(null, 1)).toBe(0);
  });
});
describe('fieldStyle', () => {
  it('brightens + glows with intensity, stays faint for ghosts', () => {
    const hi = fieldStyle(1, false), lo = fieldStyle(0, false), gh = fieldStyle(1, true);
    expect(hi.fillOpacity).toBeGreaterThan(lo.fillOpacity);
    expect(hi.glowPx).toBeGreaterThan(lo.glowPx);
    expect(gh.glowPx).toBe(0);
  });
});
describe('labeledRegionIds', () => {
  it('labels the top-K by salience', () => {
    const ids = labeledRegionIds([{id:'a',salience:0.1},{id:'b',salience:0.9},{id:'c',salience:0.5}], 2);
    expect(ids.has('b')).toBe(true); expect(ids.has('c')).toBe(true); expect(ids.has('a')).toBe(false);
  });
});
```

- [ ] **Step 2: Run RED** — `bunx vitest run src/lib/graph/atlas/labels.test.ts`. Expected: FAIL.

- [ ] **Step 3: Implement the three helpers** (append to `labels.ts`)

```ts
/** Salience → field intensity (0..1). Exponent > 1 widens the salient/tail separation. */
export function intensityOf(salience: number | null, maxSalience: number): number {
	if (maxSalience <= 0) return 0;
	return Math.pow(Math.min(1, (salience ?? 0) / maxSalience), 1.4);
}

/** Field-effect style from intensity: brighter fill/stroke + wider glow for salient regions. */
export function fieldStyle(intensity: number, ghost: boolean) {
	if (ghost) return { fillOpacity: 0.04, strokeOpacity: 0.2, glowPx: 0 };
	return {
		fillOpacity: 0.05 + intensity * 0.3,
		strokeOpacity: 0.25 + intensity * 0.5,
		glowPx: 1 + intensity * 11
	};
}

/** The top-K regions by salience — the ones that draw an in-panorama label. */
export function labeledRegionIds(
	regions: { id: string; salience: number | null }[],
	k: number
): Set<string> {
	return new Set(
		[...regions]
			.sort((a, b) => (b.salience ?? 0) - (a.salience ?? 0))
			.slice(0, k)
			.map((r) => r.id)
	);
}
```

- [ ] **Step 4: Run GREEN** — same vitest command. Expected: PASS.
- [ ] **Step 5: Controller commits** — `git commit -m "feat(atlas): salience field-effect + label-gating helpers"`.

---

## Task 4: `forceTerritories` deterministic layout + determinism test

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/layout/forceTerritories.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/forceTerritories.test.ts`

**Interfaces:**
- Consumes: `Territory[]`, `{width, height}`.
- Produces: `forceTerritories(territories, size): PositionedTerritory[]` (same shape as `packTerritories`, so `TierPanorama`'s `territoryPos`/bridges are unchanged).

- [ ] **Step 1: Write failing tests** (determinism + containment)

```ts
import { describe, it, expect } from 'vitest';
import { forceTerritories } from './forceTerritories';
const T = (id: string, salience: number) => ({ id, kind: 'region' as const, label: null, member_count: 1, salience, anchor_id: 'c', centroid: null } as any);
describe('forceTerritories', () => {
  const terr = [T('a', 1.5), T('b', 0.4), T('c', 0.9)];
  const size = { width: 800, height: 400 };
  it('is deterministic (same input → identical positions)', () => {
    expect(forceTerritories(terr, size)).toEqual(forceTerritories(terr, size));
  });
  it('sizes by salience (a > c > b)', () => {
    const r = Object.fromEntries(forceTerritories(terr, size).map((p) => [p.id, p.r]));
    expect(r.a).toBeGreaterThan(r.c); expect(r.c).toBeGreaterThan(r.b);
  });
  it('returns one positioned entry per territory', () => {
    expect(forceTerritories(terr, size)).toHaveLength(3);
  });
});
```

- [ ] **Step 2: Run RED** — `bunx vitest run src/lib/graph/atlas/layout/forceTerritories.test.ts`. Expected: FAIL.

- [ ] **Step 3: Implement `forceTerritories`**

```ts
import { forceCenter, forceCollide, forceManyBody, forceSimulation, forceX, forceY, type SimulationNodeDatum } from 'd3-force';
import type { Territory } from '$lib/types/generated/graph_territory';
import type { PositionedTerritory } from './packTerritories';

const TICKS = 300;
const LABEL_BAND = 26;
const R_MIN = 11;
const R_MAX = 42;

interface SimTerritory extends SimulationNodeDatum, PositionedTerritory {}

function territoryRadius(t: Territory, maxWeight: number): number {
	const weight = t.kind === 'region' ? (t.salience ?? 0) : Math.max(1, t.member_count);
	const norm = maxWeight > 0 ? Math.sqrt(weight / maxWeight) : 0;
	return R_MIN + norm * (R_MAX - R_MIN);
}

export function forceTerritories(territories: Territory[], size: { width: number; height: number }): PositionedTerritory[] {
	if (territories.length === 0) return [];
	const maxWeight = Math.max(...territories.map((t) => (t.kind === 'region' ? (t.salience ?? 0) : Math.max(1, t.member_count))));
	const n = territories.length;
	const cx = size.width / 2, cy = size.height / 2;
	const spread = Math.min(size.width, size.height) * 0.42;
	const nodes: SimTerritory[] = territories.map((t, i) => ({
		id: t.id, kind: t.kind, label: t.label, anchorId: t.anchor_id,
		salience: t.salience, member_count: t.member_count, r: territoryRadius(t, maxWeight),
		x: cx + Math.cos((i / Math.max(1, n)) * 2 * Math.PI) * spread,
		y: cy + Math.sin((i / Math.max(1, n)) * 2 * Math.PI) * spread
	}));
	const sim = forceSimulation(nodes)
		.force('charge', forceManyBody().strength(-40))
		.force('center', forceCenter(cx, cy))
		.force('x', forceX(cx).strength(0.04))
		.force('y', forceY(cy).strength(0.06))
		.force('collide', forceCollide<SimTerritory>().radius((d) => d.r + LABEL_BAND / 2 + 3))
		.stop();
	for (let i = 0; i < TICKS; i++) sim.tick();
	return nodes.map((d) => ({ id: d.id, kind: d.kind, label: d.label, anchorId: d.anchorId, x: d.x, y: d.y, r: d.r, salience: d.salience, member_count: d.member_count }));
}
```

- [ ] **Step 4: Run GREEN** — same vitest command. Expected: PASS.
- [ ] **Step 5: Controller commits** — `git commit -m "feat(atlas): deterministic force-separated territory layout"`.

---

## Task 5: `TerritoryCircle` field-effect + below-circle label

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte`

**Interfaces:**
- Consumes: `wrapLabel`, `fieldStyle` (Tasks 2–3). New props `intensity?: number`, `showLabel?: boolean`.

- [ ] **Step 1: Rewrite the component script** to use the helpers

```svelte
<script lang="ts">
	import type { Territory } from '$lib/types/generated/graph_territory';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import { wrapLabel, fieldStyle } from '$lib/graph/atlas/labels';

	interface Props {
		x: number; y: number; r: number; kind: Territory['kind'];
		label: string | null; memberCount?: number; onEnter?: () => void;
		ghost?: boolean; showLabel?: boolean; intensity?: number;
	}
	let { x, y, r, kind, label, memberCount = 0, onEnter, ghost = false, showLabel = true, intensity = 0.5 }: Props = $props();

	const tint = $derived(TERRITORY_TINTS[kind]);
	const radius = $derived(ghost ? r * 0.85 : r);
	const style = $derived(fieldStyle(intensity, ghost));
	const glow = $derived(style.glowPx > 0 ? `drop-shadow(0 0 ${style.glowPx}px ${tint})` : 'none');
	const baseLabel = $derived(label ?? (memberCount > 0 ? `Region · ${memberCount}` : null));
	const displayLabel = $derived(ghost && baseLabel ? `${baseLabel} · empty` : baseLabel);
	const perLineCap = $derived(Math.max(14, Math.floor(r / 2.4)));
	const lines = $derived(displayLabel ? wrapLabel(displayLabel, perLineCap) : []);
	const FONT = 11, LINE_H = 12;
</script>
```

- [ ] **Step 2: Rewrite the template** — glow circle, below-circle wrapped label, `<title>`

```svelte
<g class="territory atlas-focusable" role={onEnter ? 'button' : undefined} tabindex={onEnter ? 0 : undefined}
	aria-label={displayLabel ?? kind} onclick={onEnter} onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	style={onEnter ? 'cursor:pointer' : undefined}>
	{#if displayLabel}<title>{displayLabel}</title>{/if}
	<circle cx={x} cy={y} r={radius} fill={tint} fill-opacity={style.fillOpacity}
		stroke={tint} stroke-opacity={style.strokeOpacity} stroke-width="1.5"
		stroke-dasharray={ghost ? '3 5' : '6 4'} style={`filter:${glow}`} />
	{#if showLabel && lines.length > 0}
		<text x={x} y={y + radius + 11} text-anchor="middle" fill={tint}
			fill-opacity={ghost ? '0.6' : '1'} font-size={FONT} font-weight="600">
			{#each lines as line, i (i)}<tspan x={x} dy={i === 0 ? 0 : LINE_H}>{line}</tspan>{/each}
		</text>
	{/if}
	<circle class="focus-ring" cx={x} cy={y} r={radius + 4} stroke-width="2" />
</g>
```

- [ ] **Step 3: Controller typecheck + harness verify** — `cd packages/temper-ui && bun run check` (svelte-check) passes; open `/dev/atlas`, confirm labeled regions render mixed-case below circles with glow. Commit — `git commit -m "feat(atlas): TerritoryCircle salience field-effect + below-circle label"`.

---

## Task 6: `TierPanorama` — force layout, gating, intensity, sparse mixed-case

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`

**Interfaces:**
- Consumes: `forceTerritories` (Task 4), `intensityOf`, `labeledRegionIds` (Task 3).

- [ ] **Step 1: Swap layout + add gating/intensity** (script)

```ts
import { forceTerritories } from '$lib/graph/atlas/layout/forceTerritories';
import { intensityOf, labeledRegionIds } from '$lib/graph/atlas/labels';
// …
const packed = $derived(forceTerritories(overview.territories, terrBox));
const LABEL_MAX = 10;
const regions = $derived(packed.filter((t) => t.kind === 'region'));
const maxSalience = $derived(Math.max(0.0001, ...regions.map((t) => t.salience ?? 0)));
const labeledIds = $derived(labeledRegionIds(regions, LABEL_MAX));
```

- [ ] **Step 2: Pass `showLabel` + `intensity` to each `TerritoryCircle`**

```svelte
<TerritoryCircle x={t.x} y={t.y} r={t.r} kind={t.kind} label={t.label}
	memberCount={t.member_count}
	onEnter={t.kind === 'region' ? () => drillTerritory(t.id) : undefined}
	ghost={isEmptyTerritory(t)}
	showLabel={t.kind !== 'region' || labeledIds.has(t.id)}
	intensity={t.kind === 'region' ? intensityOf(t.salience, maxSalience) : 0.85} />
```

- [ ] **Step 3: Mixed-case the sparse cogmap-territory label** — in the `.cogmap-territory` `<text>`, remove `letter-spacing="1"` and `style="text-transform:uppercase"`.

- [ ] **Step 4: Controller typecheck + harness verify** — `bun run check`; `/dev/atlas` teamPanorama + cogmapPanorama show gated glowing field. Commit — `git commit -m "feat(atlas): field-effect panorama — force layout + salience gating"`.

---

## Task 7: `RegionHoverCard` + wire region hover

**Files:**
- Create: `packages/temper-ui/src/lib/components/graph/atlas/marks/RegionHoverCard.svelte`
- Modify: `TierPanorama.svelte` (hover state + render the card for the hovered region)

**Interfaces:**
- Props: `{ label: string | null; memberCount: number; salience: number | null; coherence: number | null; x: number; y: number }`.

- [ ] **Step 1: Create the card** (model on `NodeHoverCard.svelte` — read it first for the viewport-flip + styling pattern)

```svelte
<script lang="ts">
	interface Props { label: string | null; memberCount: number; salience: number | null; coherence: number | null; x: number; y: number; }
	let { label, memberCount, salience, coherence, x, y }: Props = $props();
	const pct = (v: number | null) => (v == null ? '—' : `${Math.round(v * 100)}%`);
</script>
<g transform={`translate(${x}, ${y})`} class="region-hovercard" pointer-events="none">
	<!-- foreignObject card: title + "N resources · salience P% · coherence Q%" -->
	<foreignObject x="8" y="-20" width="220" height="72">
		<div class="card">
			<div class="title">{label ?? 'Region'}</div>
			<div class="meta">{memberCount} resources · salience {pct(salience)} · coherence {pct(coherence)}</div>
			<div class="hint">click to enter →</div>
		</div>
	</foreignObject>
</g>
<style>/* match NodeHoverCard's card styling (dark, bordered, small) */</style>
```

- [ ] **Step 2: Wire hover state in `TierPanorama`** — `onmouseenter`/`onfocus` on each region `TerritoryCircle` sets `hovered = { id, x, y, … }`; render `<RegionHoverCard … />` when set; clear on leave/blur. (Pass through an `onHover` prop on `TerritoryCircle`, mirroring `onEnter`.)

- [ ] **Step 3: Controller typecheck + harness verify** — `bun run check`; hover a region on `/dev/atlas`, confirm the card shows resources·salience·coherence. Commit — `git commit -m "feat(atlas): region hover card (resources · salience · coherence)"`.

---

## Task 8: A11y list fallback + fixture enrichment

**Files:**
- Modify: `TierPanorama.svelte` (a11y list)
- Modify: `packages/temper-ui/static/dev/atlas-fixtures.json`, `scripts/sanitize-atlas-fixtures.mjs`

**Interfaces:** none new.

- [ ] **Step 1: Add a visually-available region list fallback** — a `<foreignObject>` or sibling block listing each region as an enterable link with `label · N resources · salience · coherence`, so the field has a non-spatial equivalent and every region is keyboard-reachable. (Confirm small/low-salience region marks keep `tabindex=0` via `onEnter`.)

- [ ] **Step 2: Enrich the committed synthetic fixtures** — regenerate `atlas-fixtures.json` (via the sanitize script) so `teamPanorama`/`cogmapPanorama` territories carry realistic derived `label`s **and** `coherence` values (the committed bundle predates B1). Update `scripts/sanitize-atlas-fixtures.mjs` to emit both fields. Do NOT commit any `*.local.json` capture.

- [ ] **Step 3: Controller verify + commit** — `bun run check`; `bunx vitest run` (fixture-driven tests green); `/dev/atlas` renders labels+field from the committed bundle. Commit — `git commit -m "feat(atlas): a11y region list fallback + labelled/coherent harness fixtures"`.

---

## Consolidated review (end of plan)

- [ ] **Spec + code review** — one pass (opus) against the Beat A spec: SQL visibility conjunct-for-conjunct (the derive's `resources_visible_to` LATERAL fails closed; deny-direction test genuine), field-effect matches spec formulas, determinism holds, no `console.log`, typed structs, a11y reachable.
- [ ] **Full gates (controller)** — `cargo make check`; `cargo nextest run -p <e2e-crate> --features test-db` (+ `test-e2e-embed` if any embed-gated fixture path is touched — `feedback_local_test_e2e_green_false_signal_for_embed`); `cd packages/temper-ui && bun run check && bunx vitest run`.
- [ ] **Merge main + push PR** (never merge locally — `feedback_always_push_pr_never_merge_local`; `git merge origin/main` first — `feedback_merge_main_before_pushing_pr`).

## Self-review (plan vs spec)

- Spec §3.1 derive+coherence → Task 1. §3.2 service → Task 1. §3.3 type → Task 1. §4.1 forceTerritories → Task 4. §4.2 wrapLabel → Task 2. §4.3 TerritoryCircle → Task 5. §4.4 TierPanorama gating/intensity → Tasks 3+6. §4.5 sparse mixed-case → Task 6. §4.6 RegionHoverCard → Task 7. §4.7 a11y → Task 8. §5 tests → Tasks 1–4 + fixtures Task 8. All covered. No placeholders (formulas/code inline). Type names consistent (`intensityOf`, `labeledRegionIds`, `fieldStyle`, `wrapLabel`, `forceTerritories`, `Territory.coherence`) across tasks.
