# Vault Resource View Rebuild — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every resource — including the 533 cogmap-homed ones that have no URL today — renders at a home-agnostic route, showing its full property set as a document masthead with an Atlas-convention rail of event history and edges.

**Architecture:** A new `/vault/r/[ident]` route resolves by UUID alone; the old context-shaped route 303s to it. All logic lives in two pure modules (`properties.ts`, `propertyValue.ts`); the Svelte components are declarative consumers. One Rust line makes the underlying meta read deterministic.

**Tech Stack:** SvelteKit 2 / Svelte 5 (runes: `$props`, `$derived`, `$state`), TypeScript, Vitest 3 (`environment: 'node'`), Rust/sqlx (one line).

**Spec:** `docs/superpowers/specs/2026-07-16-vault-resource-view-rebuild-design.md`
**Design artifact:** `design-system/preview/comp-resource-view.html` — the agreed render at fidelity. **Open it before writing any component.**

## Global Constraints

- **No component tests. Do not add jsdom or `@testing-library`.** `vite.config.ts` sets `environment: 'node'`; zero `.svelte` tests exist. This is the Atlas pattern: pure modules are tested, components are thin. Every task below tests a `.ts` module, never a `.svelte` file.
- **No new API endpoint.** `GET /api/resources/{id}` already returns `ResourceDetail` (row + both meta tiers). The page already calls it.
- **Never use `ContentResponse.managed_meta` / `.open_meta`.** `get_content_select` hardcodes both to `None` (`crates/temper-services/src/backend/substrate_read.rs:292-297`). They are dead fields.
- **Do not touch `crates/temper-substrate` beyond the single `ORDER BY` in Task 8.**
- **Do not re-tokenize the Atlas.** Out of scope; separate PR. New components consume tokens; Atlas is left alone.
- **Styling:** scoped `<style>` blocks using `--color-quiet-*`, `--font-serif`, `--font-mono`, `--track-label` from `src/app.css`, plus `--hue` set from `docTypeHue()`. **No Tailwind `zinc-*` in new components.**
- **Key every event row on `row.id`**, never a composite. See `src/lib/graph/atlas/trail.test.ts:46-58` — a composite key crashed TrailRail with `each_key_duplicate`.
- **Never `.catch()` an API error into an empty result.** `src/routes/(app)/vault/search/+page.server.ts:15` does this and renders a 500 as "no results". Do not copy it.
- Run `cd packages/temper-ui && bun run check` before any commit touching `.svelte`/`.ts`. `cargo make check` does NOT cover temper-ui.

## File Structure

| File | Responsibility |
|---|---|
| `src/lib/properties.ts` | **Create.** Merge both tiers → one ordered `PropertyRow[]`. Pure. |
| `src/lib/properties.test.ts` | **Create.** Ordering + merge tests. |
| `src/lib/propertyValue.ts` | **Create.** Classify a JSON value: scalar / object / array + summary label. Pure. |
| `src/lib/propertyValue.test.ts` | **Create.** Classification tests. |
| `src/lib/vault-url.ts` | **Modify.** `resourceHref` stops returning `null`. |
| `src/lib/vault-url.test.ts` | **Modify.** Cogmap-homed row now gets a path. |
| `src/lib/server/graph-reads.ts` | **Modify.** Add `resourceEdgesPath` + `readResourceEdges`. |
| `src/lib/server/graph-reads.paths.test.ts` | **Modify.** Edges path test. |
| `src/lib/types/resource-detail.ts` | **Create.** Hand-composed `ResourceDetail` TS type. |
| `src/routes/(app)/vault/r/[ident]/+page.server.ts` | **Create.** Four parallel reads. |
| `src/routes/(app)/vault/r/[ident]/+page.svelte` | **Create.** Layout A composition. |
| `src/lib/components/vault/HomeChip.svelte` | **Create.** Context-or-cogmap chip. |
| `src/lib/components/vault/PropertyValue.svelte` | **Create.** Recursive value renderer. |
| `src/lib/components/vault/PropertySet.svelte` | **Create.** Masthead property block. |
| `src/lib/components/vault/EventHistory.svelte` | **Create.** Trail rail section. |
| `src/lib/components/vault/EdgeList.svelte` | **Create.** Edges rail section. |
| `src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts` | **Replace.** 303 redirect. |
| `src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte` | **Delete.** |
| `crates/temper-substrate/src/readback/mod.rs` | **Modify.** One `ORDER BY`. |
| `crates/temper-substrate/tests/…` | **Modify/Create.** D7 regression guard. |

---

### Task 1: `properties.ts` — merge + order

**Files:**
- Create: `packages/temper-ui/src/lib/properties.ts`
- Test: `packages/temper-ui/src/lib/properties.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `type PropertyRow = { key: string; value: unknown; managed: boolean }` and `mergeProperties(managed: Record<string, unknown> | null, open: Record<string, unknown> | null, docType: string): PropertyRow[]`. Tasks 6 and 10 consume both.

**Context:** The managed/open split is a read-time projection over one flat `kb_properties` store, re-split by a Rust const of ten keys (`crates/temper-substrate/src/keys.rs:42`). We merge it back. Ordering is a UI convention (spec D2): `doc_type` first, then the ten managed keys in the fixed order below, then everything else alphabetically.

- [ ] **Step 1: Write the failing test**

```ts
// packages/temper-ui/src/lib/properties.test.ts
import { describe, expect, it } from 'vitest';
import { mergeProperties, MANAGED_KEY_ORDER } from './properties';

describe('mergeProperties', () => {
	it('puts doc_type first, always', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { zebra: 1 }, 'concept');
		expect(rows[0]).toEqual({ key: 'doc_type', value: 'concept', managed: true });
	});

	it('orders managed keys by MANAGED_KEY_ORDER, not alphabetically', () => {
		const rows = mergeProperties(
			{ 'temper-provenance': 'user-created', 'temper-stage': 'done' },
			null,
			'task'
		);
		// stage precedes provenance in MANAGED_KEY_ORDER despite sorting after it
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-stage', 'temper-provenance']);
	});

	it('orders open keys alphabetically, after all managed keys', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { zebra: 1, alpha: 2 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'temper-stage', 'alpha', 'zebra']);
	});

	it('marks managed vs open', () => {
		const rows = mergeProperties({ 'temper-stage': 'done' }, { alpha: 2 }, 'task');
		expect(rows.find((r) => r.key === 'temper-stage')!.managed).toBe(true);
		expect(rows.find((r) => r.key === 'alpha')!.managed).toBe(false);
	});

	it('sorts an unrecognized temper-* key into open, not managed', () => {
		// readback's inverse fate does the same: an unknown key lands in open.
		const rows = mergeProperties(null, { 'temper-invented': 'x', alpha: 1 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'alpha', 'temper-invented']);
		expect(rows.find((r) => r.key === 'temper-invented')!.managed).toBe(false);
	});

	it('drops null-valued keys', () => {
		const rows = mergeProperties({ 'temper-stage': null }, { alpha: null, beta: 0 }, 'task');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'beta']);
	});

	it('keeps falsy-but-present values', () => {
		const rows = mergeProperties(null, { zero: 0, empty: '', no: false }, 'fact');
		expect(rows.map((r) => r.key)).toEqual(['doc_type', 'empty', 'no', 'zero']);
	});

	it('handles both tiers absent', () => {
		expect(mergeProperties(null, null, 'kernel_landmark')).toEqual([
			{ key: 'doc_type', value: 'kernel_landmark', managed: true }
		]);
	});

	it('MANAGED_KEY_ORDER matches the substrate const', () => {
		// Mirrors MANAGED_PROPERTY_KEYS in crates/temper-substrate/src/keys.rs:42.
		expect(MANAGED_KEY_ORDER).toEqual([
			'temper-stage',
			'temper-mode',
			'temper-effort',
			'temper-status',
			'temper-seq',
			'temper-llm-model',
			'temper-llm-run',
			'temper-provenance',
			'temper-branch',
			'temper-pr'
		]);
	});
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/properties.test.ts`
Expected: FAIL — `Failed to resolve import "./properties"`

- [ ] **Step 3: Write minimal implementation**

```ts
// packages/temper-ui/src/lib/properties.ts

/**
 * One property as the page renders it. The managed/open split is a read-time
 * projection over a single flat `kb_properties` store — this merges it back.
 * `managed` survives only as a presentation hint (managed keys tint toward the
 * doc-type hue); it is not a storage fact.
 */
export interface PropertyRow {
	key: string;
	value: unknown;
	managed: boolean;
}

/**
 * The ten managed keys, in render order. Mirrors `MANAGED_PROPERTY_KEYS` in
 * `crates/temper-substrate/src/keys.rs:42`. Order here is editorial (workflow
 * state first, provenance last); the Rust const's order is not meaningful.
 *
 * A `temper-*` key NOT in this list is an open key — the same inverse fate the
 * substrate's `is_managed_property_key` applies.
 */
export const MANAGED_KEY_ORDER = [
	'temper-stage',
	'temper-mode',
	'temper-effort',
	'temper-status',
	'temper-seq',
	'temper-llm-model',
	'temper-llm-run',
	'temper-provenance',
	'temper-branch',
	'temper-pr'
] as const;

const MANAGED_RANK = new Map<string, number>(MANAGED_KEY_ORDER.map((k, i) => [k, i]));

/**
 * Merge both meta tiers into one ordered property set (spec D2):
 * `doc_type` first, then managed keys in `MANAGED_KEY_ORDER`, then open keys
 * alphabetically. Null-valued keys are dropped — the substrate never stores a
 * null property value, so a null here means "absent", not "set to nothing".
 */
export function mergeProperties(
	managed: Record<string, unknown> | null,
	open: Record<string, unknown> | null,
	docType: string
): PropertyRow[] {
	const managedRows: PropertyRow[] = [];
	const openRows: PropertyRow[] = [];

	for (const [key, value] of Object.entries(managed ?? {})) {
		if (value === null || value === undefined) continue;
		if (MANAGED_RANK.has(key)) managedRows.push({ key, value, managed: true });
		else openRows.push({ key, value, managed: false });
	}
	for (const [key, value] of Object.entries(open ?? {})) {
		if (value === null || value === undefined) continue;
		openRows.push({ key, value, managed: false });
	}

	managedRows.sort((a, b) => MANAGED_RANK.get(a.key)! - MANAGED_RANK.get(b.key)!);
	openRows.sort((a, b) => a.key.localeCompare(b.key));

	return [{ key: 'doc_type', value: docType, managed: true }, ...managedRows, ...openRows];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/properties.test.ts`
Expected: PASS — 9 passed

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/properties.ts packages/temper-ui/src/lib/properties.test.ts
git commit -m "feat(vault): merge both meta tiers into one ordered property set"
```

---

### Task 2: `propertyValue.ts` — classify a value

**Files:**
- Create: `packages/temper-ui/src/lib/propertyValue.ts`
- Test: `packages/temper-ui/src/lib/propertyValue.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `type ClassifiedValue = { kind: 'scalar'; text: string } | { kind: 'object'; entries: [string, unknown][]; summary: string } | { kind: 'array'; items: unknown[]; summary: string }` and `classifyValue(value: unknown): ClassifiedValue`. Task 5 (`PropertyValue.svelte`) consumes it.

**Context:** Spec D5 — one key = one row, always; depth is opt-in. A scalar renders inline; a non-scalar collapses to a summary (`{5 keys}`, `[2]`) and expands. This module decides *what* a value is; the component decides how to show it. All the logic that could be wrong is here, because the component cannot be tested.

- [ ] **Step 1: Write the failing test**

```ts
// packages/temper-ui/src/lib/propertyValue.test.ts
import { describe, expect, it } from 'vitest';
import { classifyValue } from './propertyValue';

describe('classifyValue', () => {
	it('classifies a string as scalar', () => {
		expect(classifyValue('resolved')).toEqual({ kind: 'scalar', text: 'resolved' });
	});

	it('classifies numbers and booleans as scalar, stringified', () => {
		expect(classifyValue(0)).toEqual({ kind: 'scalar', text: '0' });
		expect(classifyValue(false)).toEqual({ kind: 'scalar', text: 'false' });
	});

	it('classifies null as a scalar em-dash, never a crash', () => {
		expect(classifyValue(null)).toEqual({ kind: 'scalar', text: '—' });
	});

	it('classifies an object with its key count', () => {
		const v = { node_label: 'question', status: 'resolved' };
		expect(classifyValue(v)).toEqual({
			kind: 'object',
			entries: [
				['node_label', 'question'],
				['status', 'resolved']
			],
			summary: '{2 keys}'
		});
	});

	it('singularizes a one-key object', () => {
		expect(classifyValue({ a: 1 })).toMatchObject({ summary: '{1 key}' });
	});

	it('classifies an array with its length', () => {
		expect(classifyValue(['a', 'b'])).toEqual({
			kind: 'array',
			items: ['a', 'b'],
			summary: '[2]'
		});
	});

	it('treats an empty object/array as scalar — nothing to expand into', () => {
		expect(classifyValue({})).toEqual({ kind: 'scalar', text: '{}' });
		expect(classifyValue([])).toEqual({ kind: 'scalar', text: '[]' });
	});

	it('preserves insertion order of object entries', () => {
		const v = { zebra: 1, alpha: 2 };
		expect(classifyValue(v)).toMatchObject({ entries: [['zebra', 1], ['alpha', 2]] });
	});

	it('handles the real nested [theme] facet', () => {
		const facet = {
			node_label: 'theme',
			status: 'active',
			priority: 'high',
			slices_shipped: ['T1 team read', 'T2 invitations'],
			next_slice: 'T3 resource ownership transfer'
		};
		const got = classifyValue(facet);
		expect(got).toMatchObject({ kind: 'object', summary: '{5 keys}' });
		// the nested array is left raw — the component recurses into it
		expect(classifyValue((got as { entries: [string, unknown][] }).entries[3][1])).toMatchObject({
			kind: 'array',
			summary: '[2]'
		});
	});
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/propertyValue.test.ts`
Expected: FAIL — `Failed to resolve import "./propertyValue"`

- [ ] **Step 3: Write minimal implementation**

```ts
// packages/temper-ui/src/lib/propertyValue.ts

/**
 * What a property value IS, so the renderer can stay declarative (spec D5).
 * One key = one row: a scalar renders inline; an object/array collapses to a
 * summary and expands on demand. Depth is opt-in, so the property set always
 * reads as the resource's key set.
 *
 * Deliberately NOT `flattenPayload` (lib/graph/atlas/payloadRows.ts): that
 * flattens `facet` into sibling dot-path rows sitting at the same visual level
 * as `date` and `tags`, which stops the set reading as a key set. See spec D5.
 */
export type ClassifiedValue =
	| { kind: 'scalar'; text: string }
	| { kind: 'object'; entries: [string, unknown][]; summary: string }
	| { kind: 'array'; items: unknown[]; summary: string };

/** Classify one JSONB property value. Total — never throws, for any input. */
export function classifyValue(value: unknown): ClassifiedValue {
	if (value === null || value === undefined) return { kind: 'scalar', text: '—' };

	if (Array.isArray(value)) {
		if (value.length === 0) return { kind: 'scalar', text: '[]' };
		return { kind: 'array', items: value, summary: `[${value.length}]` };
	}

	if (typeof value === 'object') {
		const entries = Object.entries(value as Record<string, unknown>);
		if (entries.length === 0) return { kind: 'scalar', text: '{}' };
		return {
			kind: 'object',
			entries,
			summary: `{${entries.length} ${entries.length === 1 ? 'key' : 'keys'}}`
		};
	}

	return { kind: 'scalar', text: String(value) };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/propertyValue.test.ts`
Expected: PASS — 9 passed

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/propertyValue.ts packages/temper-ui/src/lib/propertyValue.test.ts
git commit -m "feat(vault): classify property values as scalar/object/array"
```

---

### Task 3: `resourceHref` — un-strand the 533

**Files:**
- Modify: `packages/temper-ui/src/lib/vault-url.ts:60-70`
- Test: `packages/temper-ui/src/lib/vault-url.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `resourceHref(row: ResourceRow): string` — **no longer `| null`**. `VaultGrid.svelte:81-82` and `CommandPalette.svelte:60` already call it and currently no-op on null; they will start working with no change.

**Context:** This one line is the headline fix. 533 of 2330 active resources are cogmap-homed; `resourceHref` returns `null` for every one, so `VaultGrid` lists them and silently no-ops on click. The new route needs no context.

- [ ] **Step 1: Write the failing test**

Append to `packages/temper-ui/src/lib/vault-url.test.ts` inside the existing `describe('resourceHref', …)` block (if there is no such block, add one at the end of the file):

```ts
	it('returns a ref route for a cogmap-homed row (the 533-resource fix)', () => {
		const row = makeRow({ context_owner_ref: null, context_slug: null, doc_type_name: 'concept' });
		expect(resourceHref(row)).toBe(`/vault/r/${ID}`);
	});

	it('returns the same ref route for a context-homed row', () => {
		const row = makeRow({
			context_owner_ref: '@j-cole-taylor',
			context_slug: 'temper',
			doc_type_name: 'task'
		});
		expect(resourceHref(row)).toBe(`/vault/r/${ID}`);
	});

	it('never returns null, whatever the home', () => {
		expect(resourceHref(makeRow({ context_owner_ref: null, context_slug: null }))).toBeTruthy();
	});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/vault-url.test.ts`
Expected: FAIL — `expected null to be '/vault/r/019f420c-…'`

> If `makeRow` rejects `context_owner_ref: null`, widen the helper's `Partial<ResourceRow>` usage — do not change `ResourceRow`. The generated type already declares these nullable (`types/generated/resource.ts:99-110`).

- [ ] **Step 3: Write minimal implementation**

Replace `packages/temper-ui/src/lib/vault-url.ts:60-70` entirely:

```ts
/**
 * Path to a resource, for any home. Resolution is trailing-UUID-only, so the
 * route needs nothing but the id — home is a rendered fact, not a routing
 * precondition (spec D1).
 *
 * This used to return `null` for a cogmap-homed resource (context_* are null),
 * which stranded 533 of 2330 active resources: VaultGrid listed them and
 * no-opped on click. It cannot return null now.
 */
export function resourceHref(row: ResourceRow): string {
	return `/vault/r/${row.id}`;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-ui && bunx vitest run src/lib/vault-url.test.ts`
Expected: PASS

Then check no caller still guards on null:

Run: `cd packages/temper-ui && bun run check`
Expected: no errors. If `VaultGrid.svelte` or `CommandPalette.svelte` has a now-dead `if (!href) return`, delete the dead branch.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/vault-url.ts packages/temper-ui/src/lib/vault-url.test.ts
git commit -m "fix(vault): resourceHref resolves for cogmap-homed resources

533 of 2330 active resources are cogmap-homed and returned null here, so
VaultGrid listed them and no-opped on click. Resolution is trailing-UUID-only;
the route never needed the context segments."
```

---

### Task 4: the edges read

**Files:**
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts`
- Test: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces: `resourceEdgesPath(id: string): string` and `readResourceEdges(token: string, id: string): Promise<GraphEdgeRow[]>`. Task 9 consumes `readResourceEdges`.

**Context:** `GET /api/resources/{id}/edges` returns `GraphEdgeRow[]`, already peer-denormalized (`peer_title`, `edge_kind`, `direction`, `weight`, `polarity`) — no subgraph load needed. `graph-reads.ts` is **server-only** (see its header) and already exports `readTrail`; follow its exact shape.

- [ ] **Step 1: Write the failing test**

Append to `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts`:

```ts
	it('builds the resource edges path', () => {
		expect(resourceEdgesPath('019f420c-cf01-7bc1-87c9-09684b0fa69e')).toBe(
			'/api/resources/019f420c-cf01-7bc1-87c9-09684b0fa69e/edges'
		);
	});
```

Add `resourceEdgesPath` to that file's existing import from `./graph-reads`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-ui && bunx vitest run src/lib/server/graph-reads.paths.test.ts`
Expected: FAIL — `resourceEdgesPath is not a function`

- [ ] **Step 3: Write minimal implementation**

Add to `packages/temper-ui/src/lib/server/graph-reads.ts`, next to `trailPath` (~line 49) and `readTrail` (~line 77) respectively:

```ts
export const resourceEdgesPath = (id: string): string => `/api/resources/${id}/edges`;
```

```ts
/** Edges incident to one resource. Rows are peer-denormalized — no subgraph load. */
export const readResourceEdges = (token: string, id: string): Promise<GraphEdgeRow[]> =>
	apiGet<GraphEdgeRow[]>(resourceEdgesPath(id), token);
```

Add `GraphEdgeRow` to the file's existing type imports from `$lib/types` (it is generated — verify the exact export name with `grep -rn "GraphEdgeRow" src/lib/types/generated/`; if absent, run `cargo make generate-ts-types` from the repo root first).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd packages/temper-ui && bunx vitest run src/lib/server/graph-reads.paths.test.ts && bun run check`
Expected: PASS, no type errors

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/server/graph-reads.ts packages/temper-ui/src/lib/server/graph-reads.paths.test.ts
git commit -m "feat(vault): add resource edges reader"
```

---

### Task 5: `ResourceDetail` TS type + `HomeChip`

**Files:**
- Create: `packages/temper-ui/src/lib/types/resource-detail.ts`
- Create: `packages/temper-ui/src/lib/components/vault/HomeChip.svelte`

**Interfaces:**
- Consumes: `ResourceRow`, `ManagedMeta`, `JsonValue` from `$lib/types`.
- Produces: `type ResourceDetail` (Task 9 and 10 consume it) and `HomeChip` with props `{ row: ResourceRow }`.

**Context:** `ResourceDetail` uses `#[serde(flatten)]`, which `ts-rs` cannot codegen — so we compose the generated halves by hand. This is not re-declaring a shape; both halves stay generated. `HomeChip` is where the 23% finding becomes visible: home renders as a fact.

- [ ] **Step 1: Write the type**

```ts
// packages/temper-ui/src/lib/types/resource-detail.ts
import type { ResourceRow, ManagedMeta } from './generated/resource';
import type { JsonValue } from './generated/serde_json/JsonValue';

/**
 * What `GET /api/resources/{id}` actually returns: the row plus both meta tiers.
 *
 * Hand-composed, deliberately. The Rust `ResourceDetail` uses `#[serde(flatten)]`,
 * which ts-rs cannot generate. Both halves here ARE generated — only the join is
 * by hand — so this does not re-declare a wire shape.
 *
 * The page previously typed this response as `ResourceRow`, which silently
 * discarded both tiers: a type assertion on a fetch result gets no excess-property
 * check. That is why the vault has never rendered frontmatter.
 */
export type ResourceDetail = ResourceRow & {
	managed_meta: ManagedMeta | null;
	open_meta: JsonValue | null;
};
```

> Verify the `JsonValue` import path with `ls src/lib/types/generated/serde_json/`. If the barrel `$lib/types` already re-exports both, import from there instead and drop the deep paths.

- [ ] **Step 2: Write `HomeChip.svelte`**

```svelte
<!-- packages/temper-ui/src/lib/components/vault/HomeChip.svelte -->
<script lang="ts">
	import type { ResourceRow } from '$lib/types';
	import { contextHref } from '$lib/vault-url';

	let { row }: { row: ResourceRow } = $props();

	// A resource is homed by exactly one anchor: a context or a cogmap
	// (kb_resource_homes.anchor_table). Cogmap-homed rows carry null context_*.
	let isCogmap = $derived(row.cogmap_id !== null);
	let label = $derived(
		isCogmap
			? (row.cogmap_name ?? 'cogmap')
			: (row.context_name ?? row.context_slug ?? 'context')
	);
	let href = $derived(
		!isCogmap && row.context_owner_ref && row.context_slug
			? contextHref(row.context_owner_ref, row.context_slug)
			: null
	);
</script>

{#if href}
	<a class="chip" {href}>◆ CONTEXT · {label}</a>
{:else}
	<span class="chip">{isCogmap ? '◈ COGMAP' : '◆ CONTEXT'} · {label}</span>
{/if}

<style>
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: 0.08em;
		padding: 2px 7px;
		border-radius: 2px;
		text-decoration: none;
		border: 1px solid color-mix(in srgb, var(--hue) 40%, transparent);
		color: color-mix(in srgb, var(--hue) 72%, white);
		background: color-mix(in srgb, var(--hue) 8%, transparent);
	}
	a.chip:hover {
		border-color: color-mix(in srgb, var(--hue) 70%, transparent);
	}
</style>
```

- [ ] **Step 3: Verify types**

Run: `cd packages/temper-ui && bun run check`
Expected: no errors

> If `svelte-check` reds on `d3-*` "implicit any" / "cannot find package" in `graph/atlas/layout/*`, that is a stale `node_modules` — run `bun install` first. It is not your change.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/types/resource-detail.ts packages/temper-ui/src/lib/components/vault/HomeChip.svelte
git commit -m "feat(vault): ResourceDetail type + HomeChip

The page typed GET /api/resources/{id} as ResourceRow and discarded both meta
tiers it already returns. HomeChip renders home as a fact rather than a routing
precondition."
```

---

### Task 6: `PropertyValue.svelte` + `PropertySet.svelte`

**Files:**
- Create: `packages/temper-ui/src/lib/components/vault/PropertyValue.svelte`
- Create: `packages/temper-ui/src/lib/components/vault/PropertySet.svelte`

**Interfaces:**
- Consumes: `classifyValue`/`ClassifiedValue` (Task 2), `PropertyRow`/`mergeProperties` (Task 1).
- Produces: `PropertySet` with props `{ rows: PropertyRow[] }`. Task 10 consumes it.

**Context:** Spec D5, artifact `design-system/preview/comp-resource-view.html` — **open it now.** `PropertyValue` recurses; Svelte 5 allows self-import by filename. These components compute nothing: `classifyValue` decides, they render.

- [ ] **Step 1: Write `PropertyValue.svelte`**

```svelte
<!-- packages/temper-ui/src/lib/components/vault/PropertyValue.svelte -->
<script lang="ts">
	import { classifyValue } from '$lib/propertyValue';
	import Self from './PropertyValue.svelte';

	let { value }: { value: unknown } = $props();

	let v = $derived(classifyValue(value));
	let open = $state(false);
</script>

{#if v.kind === 'scalar'}
	<span class="scalar">{v.text}</span>
{:else}
	<button
		class="toggle"
		aria-expanded={open}
		onclick={() => (open = !open)}
	>{open ? '⌄' : '›'} {v.summary}</button>
	{#if open}
		<div class="sub">
			{#if v.kind === 'object'}
				{#each v.entries as [k, child] (k)}
					<div class="row">
						<span class="k">{k}</span>
						<span class="v"><Self value={child} /></span>
					</div>
				{/each}
			{:else}
				{#each v.items as item, i (i)}
					<div class="item"><span class="i">{i}</span><Self value={item} /></div>
				{/each}
			{/if}
		</div>
	{/if}
{/if}

<style>
	.scalar {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-mid);
		word-break: break-word;
	}
	.toggle {
		font-family: var(--font-mono);
		font-size: 11px;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		color: color-mix(in srgb, var(--hue) 70%, white);
	}
	.toggle:hover {
		color: color-mix(in srgb, var(--hue) 90%, white);
	}
	.sub {
		border-left: 1px solid color-mix(in srgb, var(--hue) 25%, transparent);
		margin: 5px 0 5px 3px;
		padding-left: 11px;
	}
	.row {
		display: grid;
		grid-template-columns: 116px 1fr;
		gap: 8px;
		padding: 2px 0;
		align-items: start;
	}
	.k {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-dim);
	}
	.v {
		min-width: 0;
	}
	.item {
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--color-quiet-mid);
		padding: 2px 0;
	}
	.item .i {
		color: var(--color-quiet-dim);
		margin-right: 7px;
	}
</style>
```

- [ ] **Step 2: Write `PropertySet.svelte`**

```svelte
<!-- packages/temper-ui/src/lib/components/vault/PropertySet.svelte -->
<script lang="ts">
	import type { PropertyRow } from '$lib/properties';
	import PropertyValue from './PropertyValue.svelte';

	let { rows }: { rows: PropertyRow[] } = $props();

	// The rule between the managed run and the open run. Managed keys always
	// lead (mergeProperties guarantees the order), so this is the first open row.
	let firstOpenKey = $derived(rows.find((r) => !r.managed)?.key ?? null);
</script>

<div class="props">
	<div class="label">Properties · {rows.length}</div>
	<dl>
		{#each rows as row (row.key)}
			{#if row.key === firstOpenKey}
				<hr />
			{/if}
			<div class="row" class:is-managed={row.managed}>
				<dt>{row.key}</dt>
				<dd><PropertyValue value={row.value} /></dd>
			</div>
		{/each}
	</dl>
</div>

<style>
	.props {
		padding: 14px 22px 16px;
		border-bottom: 1px solid var(--color-quiet-rule);
		background: rgba(255, 255, 255, 0.015);
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 9px;
	}
	dl {
		margin: 0;
	}
	.row {
		display: grid;
		grid-template-columns: 132px 1fr;
		gap: 10px;
		padding: 3px 0;
		align-items: start;
	}
	dt {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-dim);
	}
	dd {
		margin: 0;
		min-width: 0;
	}
	/* Managed keys tint toward the doc-type hue; open keys stay neutral (spec D2). */
	.row.is-managed dt {
		color: color-mix(in srgb, var(--hue) 52%, var(--color-quiet-dim));
	}
	hr {
		border: 0;
		border-top: 1px dashed rgba(255, 255, 255, 0.1);
		margin: 8px 0;
	}
</style>
```

- [ ] **Step 3: Verify**

Run: `cd packages/temper-ui && bun run check`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/components/vault/PropertyValue.svelte packages/temper-ui/src/lib/components/vault/PropertySet.svelte
git commit -m "feat(vault): property set masthead with recursive value renderer"
```

---

### Task 7: `EventHistory.svelte` + `EdgeList.svelte`

**Files:**
- Create: `packages/temper-ui/src/lib/components/vault/EventHistory.svelte`
- Create: `packages/temper-ui/src/lib/components/vault/EdgeList.svelte`

**Interfaces:**
- Consumes: `trailModel`/`TrailRow` from `$lib/graph/atlas/trail`, `summarizeEvent` from `$lib/graph/atlas/eventSummary`, `flattenPayload` from `$lib/graph/atlas/payloadRows`, `relativeTime` from `$lib/graph/atlas/relativeTime`, `EventTrail` + `GraphEdgeRow` from `$lib/types`.
- Produces: `EventHistory` with props `{ trail: EventTrail | null }`; `EdgeList` with props `{ edges: GraphEdgeRow[] }`. Task 10 consumes both.

**Context:** Adapted from `TrailRail.svelte:129-160`. **Read that first.** The four helper modules are pure, tested, and reused verbatim — do not reimplement them. Here `flattenPayload` *is* the right tool: an event payload is a debugging dump, not the resource's key set (that distinction is spec D5).

`summarizeEvent` takes a `nodesById` map to resolve edge targets; we have no subgraph, so pass an empty `Map` — it degrades to `null` and the summary line is skipped.

- [ ] **Step 1: Write `EventHistory.svelte`**

```svelte
<!-- packages/temper-ui/src/lib/components/vault/EventHistory.svelte -->
<script lang="ts">
	import type { EventTrail } from '$lib/types';
	import { trailModel } from '$lib/graph/atlas/trail';
	import { summarizeEvent } from '$lib/graph/atlas/eventSummary';
	import { flattenPayload } from '$lib/graph/atlas/payloadRows';
	import { relativeTime } from '$lib/graph/atlas/relativeTime';

	let { trail }: { trail: EventTrail | null } = $props();

	// trailModel takes a non-null EventTrail — guard here, don't widen it.
	let rows = $derived(trail ? trailModel(trail) : []);
	let openEvent = $state<string | null>(null);

	// summarizeEvent resolves relationship targets through a node map; the vault
	// page loads no subgraph, so it degrades to null and the line is skipped.
	const NO_NODES = new Map<string, string>();
</script>

<section>
	<div class="label">History · {rows.length}</div>
	{#if rows.length === 0}
		<p class="empty">No recorded history.</p>
	{:else}
		{#each rows.slice(0, 50) as row (row.id)}
			{@const summary = summarizeEvent(row.rawKind, row.payload, NO_NODES)}
			<div class="event">
				<button
					class="head"
					aria-expanded={openEvent === row.id}
					onclick={() => (openEvent = openEvent === row.id ? null : row.id)}
				>
					<span class="kind">{row.kind}</span>
					<span class="chev">{openEvent === row.id ? '⌄' : '›'}</span>
				</button>
				{#if summary}<div class="summary">{summary}</div>{/if}
				<div class="meta">
					{row.actorName} · {relativeTime(row.occurredAt)}{#if row.confidence}
						· <span class="conf">{row.confidence}</span>{/if}
				</div>
				{#if openEvent === row.id}
					<dl class="payload">
						{#each flattenPayload(row.payload) as pr (pr.key)}
							<div><dt>{pr.key}</dt><dd>{pr.value}</dd></div>
						{/each}
					</dl>
				{/if}
			</div>
		{/each}
	{/if}
</section>

<style>
	section {
		padding: 12px 14px;
		border-top: 1px solid var(--color-quiet-rule);
	}
	section:first-child {
		border-top: 0;
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 6px;
	}
	.empty {
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 11px;
		color: var(--color-quiet-dim);
		margin: 0;
	}
	.event {
		padding: 4px 0;
	}
	.head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		width: 100%;
		background: none;
		border: 0;
		padding: 0;
		cursor: pointer;
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: color-mix(in srgb, var(--hue) 70%, white);
	}
	.chev {
		color: var(--color-quiet-dim);
	}
	.summary {
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 11px;
		color: var(--color-quiet-mid);
		margin: 1px 0;
	}
	.meta {
		font-family: var(--font-mono);
		font-size: 9px;
		color: var(--color-quiet-dim);
	}
	.conf {
		color: #8fd8a8;
	}
	.payload {
		margin: 4px 0 0;
		border-left: 1px solid color-mix(in srgb, var(--hue) 25%, transparent);
		padding-left: 8px;
	}
	.payload div {
		display: grid;
		grid-template-columns: 84px 1fr;
		gap: 6px;
	}
	.payload dt,
	.payload dd {
		font-family: var(--font-mono);
		font-size: 9px;
		margin: 0;
		word-break: break-word;
	}
	.payload dt {
		color: var(--color-quiet-dim);
	}
	.payload dd {
		color: var(--color-quiet-mid);
	}
</style>
```

> **Key on `row.id`.** `trail.test.ts:46-58` documents the `each_key_duplicate` crash from keying on actor+time+kind — two batch events collide. This is the bug that made "TrailRail fails on most resources".

- [ ] **Step 2: Write `EdgeList.svelte`**

```svelte
<!-- packages/temper-ui/src/lib/components/vault/EdgeList.svelte -->
<script lang="ts">
	import type { GraphEdgeRow } from '$lib/types';

	let { edges }: { edges: GraphEdgeRow[] } = $props();
</script>

{#if edges.length > 0}
	<section>
		<div class="label">Edges · {edges.length}</div>
		{#each edges as edge (edge.edge_id)}
			<div class="edge">
				<span class="rel">
					{edge.direction === 'out' ? '' : '← '}{edge.label ?? edge.edge_kind}{edge.direction ===
					'out'
						? ' →'
						: ''}
				</span>
				<a class="peer" href="/vault/r/{edge.peer_resource_id}">{edge.peer_title}</a>
				<span class="w">
					· {edge.weight.toFixed(1)}{#if edge.polarity && edge.polarity !== 'positive'}
						· {edge.polarity}{/if}
				</span>
			</div>
		{/each}
	</section>
{/if}

<style>
	section {
		padding: 12px 14px;
		border-top: 1px solid var(--color-quiet-rule);
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 6px;
	}
	.edge {
		padding: 4px 0;
		font-family: var(--font-mono);
		font-size: 10.5px;
	}
	.rel {
		color: var(--color-quiet-dim);
	}
	.peer {
		color: var(--color-quiet-mid);
		text-decoration: none;
	}
	.peer:hover {
		color: var(--color-quiet-fg);
		text-decoration: underline;
	}
	.w {
		color: var(--color-quiet-dim);
		font-size: 9px;
	}
</style>
```

> **Verify `GraphEdgeRow`'s field names and `direction`'s exact values before running** — `grep -n "direction\|polarity\|edge_id\|peer_" src/lib/types/generated/graph.ts`. Fix the template to match; do not change the generated type. Empty edges render nothing (Atlas convention); History is the only section that announces its emptiness.

- [ ] **Step 3: Verify**

Run: `cd packages/temper-ui && bun run check`
Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/components/vault/EventHistory.svelte packages/temper-ui/src/lib/components/vault/EdgeList.svelte
git commit -m "feat(vault): event history + edge list rail sections

Adapted from TrailRail's history section; reuses trail/eventSummary/payloadRows/
relativeTime verbatim. Edges render weight and polarity — they are carried, and
an author who set a weight meant it."
```

---

### Task 8: the `ORDER BY` — make the meta read deterministic

**Files:**
- Modify: `crates/temper-substrate/src/readback/mod.rs:241-245`
- Test: `crates/temper-api/tests/resource_update_merge_test.rs` (append)

**Interfaces:**
- Consumes: nothing.
- Produces: nothing new — `readback::meta`'s signature is unchanged. Behaviour becomes deterministic.

**Context:** Spec D7. The query has no `ORDER BY` and the loop map-inserts per row, so with two non-folded rows for one key, *which one survives is whatever order Postgres returns*. In production this hits 13 resources, all cogmap-homed: a resolved question can render as open, differently between two page loads. Verified: `ORDER BY created` disambiguates every pair with zero timestamp ties.

This is bundled here rather than extracted because this work surfaced it and the property view is unshippable on a non-deterministic read — the repo's stated convention for bundling.

**This does not fix facet supersession** — that is task `019f6d08-2b55-7ee0-b9ac-1959cf4d736b`. Do not touch `facet_set` or fold anything.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/tests/resource_update_merge_test.rs`. That file is the right home: it already exercises `readback::meta` through the real API path via its `fetch_open_meta` helper, and its own docstring says so ("*the substrate reconstructs it from kb_properties via readback::meta*") — this proves the fix at the production caller's level rather than at the unit.

The two rows must be inserted with **raw SQL**, not through the API: the meta PUT path uses `property_set`, which folds-then-inserts, so it can never produce the two-live-rows state. Only `facet_set` appends, and that is what created the 13 production rows.

```rust
/// Two non-folded rows for one key collapse to the NEWEST, deterministically.
///
/// `readback::meta`'s query had no ORDER BY and the reader map-inserts per row,
/// so the survivor was whatever order Postgres returned. 13 production resources
/// are in this state — all cogmap-homed, all facets updated via `facet_set`,
/// which appends rather than folding. A resolved question could read as open,
/// and differently between two reads.
///
/// Seeded with raw SQL on purpose: the API's meta path uses `property_set`
/// (fold-then-insert) and cannot reach this state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_collapses_a_repeated_key_to_the_newest_row(pool: PgPool) {
    let app = common::TestApp::new(pool.clone()).await;
    let (token, resource_id_str) = setup_resource_with_open_meta(&app, json!({ "date": "2026-07-03" })).await;
    let resource_id = uuid::Uuid::parse_str(&resource_id_str).expect("resource id");

    // Both FKs are NOT NULL REFERENCES kb_events(id); the create already emitted one.
    let event_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("an event exists after resource creation");

    for (created, status) in [
        ("2026-07-03T10:00:00Z", "open"),
        ("2026-07-03T11:00:00Z", "resolved"),
    ] {
        sqlx::query(
            "INSERT INTO kb_properties
                 (owner_table, owner_id, property_key, property_value, weight,
                  asserted_by_event_id, last_event_id, is_folded, created)
             VALUES ('kb_resources', $1, 'facet', $2, 1.0, $3, $3, false, $4::timestamptz)",
        )
        .bind(resource_id)
        .bind(json!({ "status": status }))
        .bind(event_id)
        .bind(created)
        .execute(&pool)
        .await
        .expect("seed facet row");
    }

    // Repeated: an unordered read can return insertion order by luck.
    for _ in 0..5 {
        let open = fetch_open_meta(&app, &token, &resource_id_str).await;
        assert_eq!(
            open["facet"],
            json!({ "status": "resolved" }),
            "readback::meta must return the newest row for a repeated key"
        );
    }
}
```

> Match the file's existing helper signatures — check what `setup_resource_with_open_meta` (line ~97) actually returns and destructure accordingly; the tuple above is the expected shape, not a verified one. Add `use uuid::Uuid;` / `use sqlx::PgPool;` only if the file lacks them.

These are runtime `sqlx::query` / `query_scalar` (no `!`), so **no `.sqlx` cache regeneration is needed** — this is fixture setup, which the repo's rules explicitly allow to be runtime. If you reach for `query!` here you have made work for yourself: you would then owe `cargo make prepare-api`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo make docker-up
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo nextest run -p temper-api --features test-db --test resource_update_merge_test
```
Expected: FAIL — asserts `{"status":"open"}` where `{"status":"resolved"}` was expected, on at least one of the five reads.

> **Never run a bare `cargo nextest run -p temper-api`** — it hangs at test-list enumeration on the bin target. Always scope with `--test <target>`.
>
> **If it passes on the first run, do not conclude the bug is absent.** An unordered read can return insertion order by luck; the 5-iteration loop lowers that odds but cannot eliminate it. The proof is the SQL: read `readback/mod.rs:241-245` and confirm there is no `ORDER BY`. This is a determinism bug — a green test is weak evidence in *both* directions, which is exactly why it survived in production.

- [ ] **Step 3: Write minimal implementation**

In `crates/temper-substrate/src/readback/mod.rs`, change the query at ~line 241:

```rust
    let rows = sqlx::query(
        "SELECT property_key, property_value
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = $1 AND NOT is_folded
          ORDER BY created, id",
    )
```

Add above the `for row in &rows` loop:

```rust
    // ORDER BY created, id + last-write-wins on the inserts below = newest wins.
    // `facet_set` appends, so an updated facet leaves the superseded row live
    // (13 resources in production). Without the ordering, which one survived was
    // whatever order Postgres returned. `id` is a uuidv7 tiebreak for rows written
    // in one transaction. Facet supersession itself is task
    // 019f6d08-2b55-7ee0-b9ac-1959cf4d736b — this only makes the READ honest.
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo nextest run -p temper-api --features test-db --test resource_update_merge_test
```
Expected: PASS — all tests in the target, including the pre-existing merge tests (the ordering must not disturb them).

The changed query is a runtime `sqlx::query`, not a macro, so **no `.sqlx` regeneration is needed**. Confirm:
```bash
cargo make check
```
Expected: passes. If it reds on a missing cache entry, you changed a macro query by mistake — revert and re-read the module header ("Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros)").

- [ ] **Step 5: Commit**

```bash
git add crates/temper-substrate/src/readback/mod.rs crates/temper-api/tests/resource_update_merge_test.rs
git commit -m "fix(readback): order kb_properties reads so same-key collapse is deterministic

readback::meta had no ORDER BY and map-inserts per row, so with two non-folded
rows for one key the survivor was whatever order Postgres returned. 13 production
resources are affected, all cogmap-homed: a resolved question could render as
open, differently between two page loads.

ORDER BY created, id + last-write-wins = newest wins, matching author intent in
all 13. Facet supersession itself is a separate task."
```

---

### Task 9: the route + server load

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/vault/r/[ident]/+page.server.ts`
- Modify: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts` (replace wholesale)
- Delete: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte`

**Interfaces:**
- Consumes: `ResourceDetail` (Task 5), `readResourceEdges` (Task 4), `parseRef`, `apiGet`/`ApiError`, `readTrail`.
- Produces: `PageData = { resource: ResourceDetail; content: string; trail: EventTrail | null; edges: GraphEdgeRow[] }`. Task 10 consumes it.

**Context:** Spec D1. Four parallel reads. SvelteKit gives static segments priority, so `/vault/r/[ident]` wins over `/vault/[owner]/[context]` — and owner refs are `@handle`/`+team-slug`, so a bare `r` can never be one.

- [ ] **Step 1: Write the new loader**

```ts
// packages/temper-ui/src/routes/(app)/vault/r/[ident]/+page.server.ts
import type { PageServerLoad } from './$types';
import { error } from '@sveltejs/kit';
import { apiGet, ApiError } from '$lib/server/api';
import { readTrail, readResourceEdges } from '$lib/server/graph-reads';
import { parseRef } from '$lib/ref';
import type { ResourceDetail } from '$lib/types/resource-detail';
import type { EventTrail, GraphEdgeRow } from '$lib/types';

export const load: PageServerLoad = async ({ locals, params }) => {
	const accessToken = locals.accessToken!;
	const id = parseRef(params.ident);

	// GET /api/resources/{id} returns ResourceDetail — the row AND both meta
	// tiers. Do NOT read the tiers off /content: get_content_select hardcodes
	// both to None (substrate_read.rs:292-297). They are dead fields.
	let resource: ResourceDetail;
	try {
		resource = await apiGet<ResourceDetail>(`/api/resources/${id}`, accessToken);
	} catch (err) {
		if (err instanceof ApiError && err.status === 404) throw error(404, 'Resource not found');
		throw err;
	}

	// The rail degrades independently: a failure here must not blank the body.
	// Narrow to the rail reads only — never wrap the resource/content reads in
	// a catch that turns an API error into an empty render.
	const [content, trail, edges] = await Promise.all([
		apiGet<{ markdown: string }>(`/api/resources/${id}/content`, accessToken).then((r) => r.markdown),
		readTrail(accessToken, 'node', id).catch((): EventTrail | null => null),
		readResourceEdges(accessToken, id).catch((): GraphEdgeRow[] => [])
	]);

	return { resource, content, trail, edges };
};
```

- [ ] **Step 2: Replace the old route with a redirect**

Replace `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts` entirely:

```ts
import type { PageServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { parseRef } from '$lib/ref';

/**
 * Legacy context-shaped resource URL. Resolution was always trailing-UUID-only,
 * so the owner/context/doc_type segments never carried meaning — and presuming a
 * context home left 533 cogmap-homed resources unaddressable (spec D1). Alias to
 * the home-agnostic route; existing links and bookmarks keep working.
 */
export const load: PageServerLoad = async ({ params }) => {
	redirect(303, `/vault/r/${parseRef(params.ident)}`);
};
```

Then delete the old page component:

```bash
rm "packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte"
```

- [ ] **Step 3: Verify**

Run: `cd packages/temper-ui && bun run check`
Expected: no errors. `ResourceMetaHeader.svelte` may now be unused — check with `grep -rn "ResourceMetaHeader" src/`. If it has no remaining importer, delete it and its file; a dead component is a trap for the next reader.

- [ ] **Step 4: Commit**

```bash
git add -A packages/temper-ui/src/routes/\(app\)/vault/
git commit -m "feat(vault): home-agnostic /vault/r/[ident] route

Resolution was always trailing-UUID-only; the context segments were presentation
that happened to gate 533 cogmap-homed resources out of the vault entirely. The
old route 303s here, so links keep working."
```

---

### Task 10: the page — layout A composition

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/vault/r/[ident]/+page.svelte`

**Interfaces:**
- Consumes: everything from Tasks 1, 2, 5, 6, 7, 9.
- Produces: the page.

**Context:** Spec D4 + the artifact `design-system/preview/comp-resource-view.html`. **Open the artifact side-by-side.** One `--hue` on the grid root tints the whole view.

- [ ] **Step 1: Write the page**

```svelte
<!-- packages/temper-ui/src/routes/(app)/vault/r/[ident]/+page.svelte -->
<script lang="ts">
	import type { PageData } from './$types';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
	import HomeChip from '$lib/components/vault/HomeChip.svelte';
	import PropertySet from '$lib/components/vault/PropertySet.svelte';
	import EventHistory from '$lib/components/vault/EventHistory.svelte';
	import EdgeList from '$lib/components/vault/EdgeList.svelte';
	import { mergeProperties } from '$lib/properties';
	import { docTypeHue } from '$lib/graph/atlas/palette';

	let { data }: { data: PageData } = $props();

	// One --hue on the root tints masthead, rules, chips, toggles and rail.
	// An unknown doc type falls back to FALLBACK_HUE — no branching needed.
	let hue = $derived(docTypeHue(data.resource.doc_type_name));

	let rows = $derived(
		mergeProperties(
			data.resource.managed_meta as Record<string, unknown> | null,
			data.resource.open_meta as Record<string, unknown> | null,
			data.resource.doc_type_name
		)
	);
</script>

<svelte:head>
	<title>{data.resource.title} — temper</title>
</svelte:head>

<div class="rv" style="--hue: {hue}">
	<div class="main">
		<div class="masthead">
			<div class="eyebrow">{data.resource.doc_type_name}</div>
			<h1 class="title">{data.resource.title}</h1>
			<HomeChip row={data.resource} />
		</div>

		<PropertySet {rows} />

		<div class="body">
			<MarkdownRenderer markdown={data.content} />
		</div>
	</div>

	<aside class="rail">
		<EventHistory trail={data.trail} />
		<EdgeList edges={data.edges} />
	</aside>
</div>

<style>
	.rv {
		display: grid;
		grid-template-columns: 1fr 260px;
		min-height: 100%;
	}
	.main {
		min-width: 0;
	}
	.masthead {
		padding: 18px 22px;
		border-bottom: 1px solid var(--color-quiet-rule);
	}
	.eyebrow {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: color-mix(in srgb, var(--hue) 80%, white);
	}
	.title {
		font-family: var(--font-serif);
		font-weight: 400;
		font-size: 25px;
		line-height: 1.25;
		letter-spacing: -0.01em;
		color: var(--hue);
		margin: 8px 0 11px;
	}
	.body {
		padding: 18px 22px 24px;
	}
	.rail {
		background: var(--color-quiet-card);
		border-left: 1px solid color-mix(in srgb, var(--hue) 22%, transparent);
	}

	@media (max-width: 900px) {
		.rv {
			grid-template-columns: 1fr;
		}
		.rail {
			border-left: 0;
			border-top: 1px solid color-mix(in srgb, var(--hue) 22%, transparent);
		}
	}
</style>
```

- [ ] **Step 2: Verify types and lint**

Run: `cd packages/temper-ui && bun run check && bunx vitest run`
Expected: no errors; all tests pass

- [ ] **Step 3: Verify the full suite**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
cargo make check
cargo nextest run -p temper-api --features test-db --test resource_update_merge_test
```
Expected: both pass

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/\(app\)/vault/r/\[ident\]/+page.svelte
git commit -m "feat(vault): resource view — editorial masthead + Atlas rail

Properties are identity, so they sit with the title before the prose; the rail
carries what happened to the resource. One --hue on the root tints the view, and
an unknown doc type falls back rather than branching."
```

---

## Verification

Because there is no component-test infrastructure and the local DB has one resource, **nothing here proves the page renders.** Be honest about that in the PR. What IS verified:

- [ ] `bunx vitest run` — properties ordering, value classification, `resourceHref` for both homes, the edges path
- [ ] `cargo nextest run -p temper-api --features test-db --test resource_update_merge_test` — the D7 determinism guard
- [ ] `cargo make check` and `cd packages/temper-ui && bun run check`
- [ ] `grep -rn "zinc-" packages/temper-ui/src/lib/components/vault/ packages/temper-ui/src/routes/\(app\)/vault/r/` returns nothing — the new surface is on the token layer
- [ ] `grep -rn "ResourceMetaHeader" packages/temper-ui/src/` returns nothing, or its remaining callers are deliberate

The durable fix for the gap is `/dev/vault` — task `019f6d08-8b33-7f30-a438-8487261d5f23`.

## Out of scope

- **Atlas re-tokenization** (spec D3's second half). A refactor of a working, untested surface; belongs in its own PR so this one tells one story.
- **Facet supersession** — task `019f6d08-2b55-7ee0-b9ac-1959cf4d736b`.
- **`ContentResponse`'s dead meta fields** — spec follow-on 3, unfiled.
- **Lineage** — `derived_from` is an edge kind; it would render twice.
