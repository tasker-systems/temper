# Working Surface Narrowing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the filters the read door serves as a URL-driven narrowing surface with kind-aware columns, and widen the door so its facet histograms stop collapsing when used.

**Architecture:** Three doc-type/stage/status predicates move out of SQL into the existing Rust filter step in `filtered_visible_page`, so each facet histogram can exclude its own predicate — the single change that makes a faceted browse UI honest, and that makes CSV multi-select a set-membership test. The UI then grows two pure modules (URL filter state, column derivation) tested with vitest, a filter bar that renders what they decide, and one shared browser component replacing three near-identical page wrappers.

**Tech Stack:** Rust (sqlx runtime `query()`, axum), SvelteKit 2 / Svelte 5 runes, vitest, biome, ts-rs codegen.

**Spec:** `docs/superpowers/specs/2026-08-18-working-surface-narrowing-design.md` — read it before Task 1. Vault copy: `01a0159b-cd6e-7e63-9352-6d85ff437924`.

## Global Constraints

- **Subagents must NOT run any `cargo` command** — not build, test, clippy, fmt, `cargo make`, or `sqlx prepare`. A cold build here takes 4–12 minutes against a 120s Bash default, so a subagent that launches one parks forever. Subagents write code and report; **the controller runs every cargo gate and commits the Rust tasks.** Read-only commands (`rg`, `cat`, `git log/diff`) are fine.
- **Subagents MAY run `bun` commands** in `packages/temper-ui` (`bun run test`, `bun run check`, `bun run biome`) — vitest and svelte-check are fast and do not have the cargo problem.
- **No `.sqlx` regeneration anywhere in this plan.** The list query is built with `format!` and executed through runtime `sqlx::query()`, documented in place as "the documented runtime-`query` exception (dynamic ORDER clause), not a static macro." No `query!` macro is touched.
- **No migration.** No schema change.
- `--all-features` for all builds and clippy.
- `#[expect(lint, reason = "...")]` never `#[allow]`.
- All public types implement `Debug`.
- Statement budget: `crates/temper-services/tests/list_page_query_count_test.rs:107` asserts `statements <= 3` for a list page. **This plan must not add a query.** That existing test is the guard.

**On why implementation steps cite `file:line` instead of carrying code bodies.** This is deliberate and is the repo's standing rule, not an under-specified plan. Invented code bodies in plans are reliably stale on arrival — they are written from a mental model rather than from disk — and worse, they *win*: an implementer builds the code block rather than the correct prose beside it. So this plan carries **exact code only where it must be exact**: test bodies (TDD needs the real assertion) and interface signatures (later tasks bind to those names). Implementation steps instead name the file, the line, and the sibling to mirror. If a step feels under-specified, open the cited line — that is the specification. Escalate as BLOCKED rather than inventing a shape the citation does not support.

---

## File Structure

**Rust — the read door**

| File | Responsibility |
|---|---|
| `crates/temper-workflow/src/types/resource.rs:143-145` | `ResourceFacets` gains `stage` and `status` maps |
| `crates/temper-services/src/backend/substrate_read.rs:122-336` | `filtered_visible_page` — predicates move, three histograms computed |
| `crates/temper-services/tests/list_facet_independence_test.rs` *(new)* | Asserts a dimension's own filter does not shrink its histogram |

**Generated artifacts** — `packages/temper-ui/src/lib/types/generated/resource.ts`, `openapi.json`, temper-rb, temper-ts.

**Svelte — the surface**

| File | Responsibility |
|---|---|
| `packages/temper-ui/src/lib/vault-filters.ts` *(new)* | URL filter state + the revealed-kind predicate. Pure. |
| `packages/temper-ui/src/lib/vault-columns.ts` *(new)* | Kind→managed-keys map + column derivation. Pure. |
| `packages/temper-ui/src/lib/components/vault/FilterBar.svelte` *(new)* | Renders controls; no logic |
| `packages/temper-ui/src/lib/components/vault/VaultBrowser.svelte` *(new)* | Shared chrome: heading + bar + chips + grid |
| `packages/temper-ui/src/lib/components/VaultGrid.svelte` | Takes `columns` prop; reads envelope paging |
| `packages/temper-ui/src/lib/components/FacetChips.svelte` | Multi-select |
| 3 × `+page.server.ts`, 3 × `+page.svelte` | Thin mounts |

---

### Task 1: Facet independence, stage/status histograms, CSV doc-type

One atomic change: the wire type, the read path, and both Rust consumers move together. Cross-crate type changes are one commit in this repo.

**Files:**
- Modify: `crates/temper-workflow/src/types/resource.rs:143-145`
- Modify: `crates/temper-services/src/backend/substrate_read.rs:122-336`
- Test: `crates/temper-services/tests/list_facet_independence_test.rs` (create)

**Interfaces:**
- Produces: `ResourceFacets { doc_type: HashMap<String,i64>, stage: HashMap<String,i64>, status: HashMap<String,i64> }`. `Default` is already derived (`resource.rs:132`) and the only struct-literal construction is inside `substrate_read.rs`; `ResourceFacets::default()` callers (`temper-cli/src/commands/resource.rs:2756`) keep compiling.
- Consumes: nothing from earlier tasks.

**Grounded facts — do not re-derive, and do not contradict:**
- The three predicates to remove are exactly `substrate_read.rs:234` (`dt.property_value #>> '{}' = $3`), `:235` (`wp.stage = $4`), `:247` (`wp.status = $11`). **Leave every other WHERE clause alone** — `context_ref`, `owner`, `q`, `goal`, `tags` and `cogmap_ids` stay in SQL because none publishes a histogram.
- `LEFT JOIN kb_resource_workflow_props wp` is **already present**. Add `wp.stage` and `wp.status` to the SELECT list; do not add a join.
- Pagination is already Rust-side (`substrate_read.rs:319-328`), so Rust-side filtering is the incumbent pattern here, not a new one.
- `validate_goal_status(status)` (`substrate_read.rs`, from `schema.rs:367`) stays exactly where it is — it validates the *value*, and is unrelated to the predicate move.
- ORDER BY stays in SQL. Filtering in Rust preserves the SQL order, so pagination is unaffected.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/tests/list_facet_independence_test.rs`. Mirror the harness of `crates/temper-services/tests/list_page_query_count_test.rs` — same `#![cfg(feature = "test-db")]` module gate, same `#[sqlx::test(migrator = "temper_services::MIGRATOR")]` attribute, and the same `seed_profile_with_context` helper shape (copy it; it is test-local in that file).

Seed one context with a mix: 3 `task` resources (stages `backlog`, `backlog`, `done`), 2 `goal` resources (status `active`, `completed`), and 1 `research`. Then assert:

```rust
// A doc-type filter must NOT shrink the doc-type histogram — the defect this change fixes.
let params = ResourceListParams {
    doc_type_name: Some("task".to_string()),
    ..Default::default()
};
let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
    .await
    .expect("list");
assert_eq!(page.total, 3, "total IS filtered");
assert_eq!(page.facets.doc_type.get("task"), Some(&3));
assert_eq!(
    page.facets.doc_type.get("goal"),
    Some(&2),
    "the doc_type histogram must exclude its own predicate"
);
assert_eq!(
    page.facets.doc_type.get("research"),
    Some(&1),
    "every other kind stays visible and reachable"
);

// The stage histogram excludes its own predicate but respects doc_type.
let params = ResourceListParams {
    doc_type_name: Some("task".to_string()),
    stage: Some("done".to_string()),
    ..Default::default()
};
let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
    .await
    .expect("list");
assert_eq!(page.total, 1, "total reflects BOTH filters");
assert_eq!(
    page.facets.stage.get("backlog"),
    Some(&2),
    "the stage histogram excludes its own predicate"
);
assert_eq!(page.facets.stage.get("done"), Some(&1));
assert!(
    !page.facets.stage.contains_key("active"),
    "stage histogram is scoped by doc_type, which is NOT its own predicate"
);

// CSV multi-select.
let params = ResourceListParams {
    doc_type_name: Some("task,goal".to_string()),
    ..Default::default()
};
let page = substrate_read::list_select(&pool, ProfileId::from(profile), params)
    .await
    .expect("list");
assert_eq!(page.total, 5, "CSV selects the union");
```

- [ ] **Step 2: Controller runs the test to verify it fails**

Run: `cargo test -p temper-services --features test-db --test list_facet_independence_test -- --nocapture > /tmp/t1.log 2>&1; tail -40 /tmp/t1.log`

Expected: FAIL — `no field \`stage\` on type \`ResourceFacets\`` (compile error), which is the correct red.

Redirect to a file rather than piping to `tail`; a pipe masks the exit code.

- [ ] **Step 3: Add the two fields to `ResourceFacets`**

In `crates/temper-workflow/src/types/resource.rs:143-145`, add `stage` and `status` as `std::collections::HashMap<String, i64>` beside `doc_type`, each with a doc comment stating that the histogram **excludes its own filter predicate** and is scoped by the others. Update the type's own doc comment (`resource.rs:137`) — it currently reads "Aggregated doc-type facet counts for the current filter set," which will no longer be the whole truth.

- [ ] **Step 4: Restructure `filtered_visible_page`**

In `crates/temper-services/src/backend/substrate_read.rs`:

1. Add `wp.stage`, `wp.status` to the SELECT list (join already present).
2. Delete the three predicates at `:234`, `:235`, `:247` from the WHERE. Renumber the remaining binds and their `.bind(...)` calls to stay contiguous — this is the fiddliest part of the task; the binds are positional.
3. Parse `params.doc_type_name` as CSV into an `Option<HashSet<String>>`, mirroring the trim/empty handling of the `tags` parser at `substrate_read.rs:186-199` (split on `,`, trim, drop empties, and treat an all-empty CSV as `None` rather than an empty set — an empty set would match nothing where absent means no filter).
4. Replace the single histogram loop at `:309-317` with three predicate closures and four passes over the returned rows:
   - `doc_type` histogram over rows matching **stage ∧ status**
   - `stage` histogram over rows matching **doc_type ∧ status**, keyed by the row's stage, skipping rows whose stage is `NULL`
   - `status` histogram over rows matching **doc_type ∧ stage**, keyed by the row's status, skipping `NULL`
   - the kept set = rows matching **all three**; `total` is its length; the page slice comes from it, preserving the existing `skip`/`take` at `:319-328`

Keep `VisiblePage` (`:72-78`) as the carrier; widen its `facets` field to hold all three maps.

Do **not** add doc-type name validation. Current behaviour accepts any string and matches nothing on a typo; changing that is out of scope and is noted as such in the spec.

- [ ] **Step 5: Controller runs the test to verify it passes**

Run: `cargo test -p temper-services --features test-db --test list_facet_independence_test > /tmp/t1.log 2>&1; tail -40 /tmp/t1.log`
Expected: PASS.

- [ ] **Step 6: Controller verifies the statement budget did not move**

Run: `cargo test -p temper-services --features test-db --test list_page_query_count_test > /tmp/t1b.log 2>&1; tail -20 /tmp/t1b.log`
Expected: PASS. This is the guard that no second query crept in. If it fails with a count above 3, the implementation reached for a probe query — reject and redo Step 4.

- [ ] **Step 7: Controller runs the crate gates and commits**

```bash
cargo fmt -p temper-services -p temper-workflow
cargo clippy -p temper-services -p temper-workflow --all-features --all-targets > /tmp/t1c.log 2>&1; tail -30 /tmp/t1c.log
git add crates/temper-workflow/src/types/resource.rs crates/temper-services/src/backend/substrate_read.rs crates/temper-services/tests/list_facet_independence_test.rs
git commit -F - <<'MSG'
feat(api): facet histograms exclude their own predicate; multi-select doc-type

The doc_type/stage/status predicates move from SQL into the existing Rust filter
step, so each histogram can be computed over the set filtered by the OTHER
predicates. Previously doc_type was filtered in SQL before the histogram was
built, so selecting a doc-type collapsed the histogram to that one key and a
browse UI could not show or reach the alternatives.

CONTRACT CHANGE (spec D5): facets.doc_type is now pre-filter. A caller that
filters by doc-type and reads facets.doc_type will see the full distribution
rather than its own selection. Additive: facets.stage and facets.status are new
fields; doc_type_name additionally accepts CSV, and a single value is unchanged.

Still one statement per page — list_page_query_count_test is the guard.
MSG
```

---

### Task 2: Regenerate the derived artifacts

**Files:**
- Modify: `packages/temper-ui/src/lib/types/generated/resource.ts` (ts-rs)
- Modify: `openapi.json`, temper-rb gem, temper-ts `schema.ts`

**Interfaces:**
- Consumes: `ResourceFacets` from Task 1.
- Produces: the TS `ResourceFacets` type that Tasks 3–7 import.

- [ ] **Step 1: Controller regenerates**

Follow the `generated-artifacts` skill. Run its regeneration commands, not hand edits — every file here is generated.

- [ ] **Step 2: Confirm the TS type carries all three maps**

Run: `rg -n "ResourceFacets" -A 4 packages/temper-ui/src/lib/types/generated/resource.ts`
Expected: `doc_type`, `stage` and `status`, each `{ [key in string]?: bigint }`.

- [ ] **Step 3: Commit every changed generated file together**

Codegen drift gates clear at different git stages — rb/ts drift clears on `git add`, ts-rs drift only after the commit — so stage and commit all of them in one go, then re-run the gate.

```bash
git add -A
git commit -m "chore: regenerate artifacts for widened ResourceFacets"
```

- [ ] **Step 4: Controller runs the workspace gate**

Run: `cargo make check > /tmp/t2.log 2>&1; tail -40 /tmp/t2.log`
Expected: PASS, including `openapi-check`, `openapi-rb-drift`, `openapi-ts-drift` and `ts-rs-drift`.

---

### Task 3: `vault-filters.ts` — URL filter state

**Files:**
- Create: `packages/temper-ui/src/lib/vault-filters.ts`
- Test: `packages/temper-ui/src/lib/vault-filters.test.ts`

**Interfaces:**
- Produces, relied on by Tasks 5 and 7:

```ts
export interface VaultFilters {
	docTypes: string[];
	stage: string | null;
	status: string | null;
	contextRef: string | null;
	q: string | null;
	tags: string[];
}
export function parseFilters(url: URL): VaultFilters;
export function buildFilterUrl(base: URL, patch: Partial<VaultFilters>): string;
export function toggleDocType(base: URL, name: string): string;
export function revealedKind(filters: VaultFilters, facets: Record<string, number>): string | null;
export function activeFilterCount(filters: VaultFilters): number;
```

**Pattern to mirror — read it first:** `packages/temper-ui/src/lib/graph/atlas/nav.ts:158-181`. It is the incumbent for URL filter state: a private `withParams(base, mutate)` returning `${pathname}${search}`, CSV encoding for multi-value params, and `p.delete(k)` for the empty case. Mirror that structure exactly. Do **not** import from `nav.ts` — it is Atlas-scoped; duplicate the four-line `withParams` helper locally, as `nav.ts` itself keeps it private.

Every mutation must `p.delete('offset')`, matching `FacetChips.svelte:28` and `VaultGrid.svelte:92` — narrowing resets to page one.

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it } from 'vitest';
import {
	activeFilterCount,
	buildFilterUrl,
	parseFilters,
	revealedKind,
	toggleDocType,
} from './vault-filters';

const at = (search: string) => new URL(`https://x.test/vault/all${search}`);

describe('parseFilters', () => {
	it('reads an empty URL as no filters', () => {
		expect(parseFilters(at(''))).toEqual({
			docTypes: [], stage: null, status: null, contextRef: null, q: null, tags: [],
		});
	});

	it('splits doc_type_name as CSV', () => {
		expect(parseFilters(at('?doc_type_name=task,goal')).docTypes).toEqual(['task', 'goal']);
	});

	it('trims CSV members and drops empties', () => {
		expect(parseFilters(at('?doc_type_name=task,%20,goal,')).docTypes).toEqual(['task', 'goal']);
	});
});

describe('buildFilterUrl', () => {
	it('resets offset whenever a filter changes', () => {
		const out = buildFilterUrl(at('?offset=100&sort=title'), { stage: 'done' });
		expect(out).not.toContain('offset');
		expect(out).toContain('sort=title');
		expect(out).toContain('stage=done');
	});

	it('deletes a param set to null rather than writing an empty value', () => {
		expect(buildFilterUrl(at('?stage=done'), { stage: null })).not.toContain('stage');
	});

	it('encodes docTypes back to CSV', () => {
		expect(buildFilterUrl(at(''), { docTypes: ['task', 'goal'] })).toContain(
			'doc_type_name=task%2Cgoal',
		);
	});
});

describe('toggleDocType', () => {
	it('adds a kind that is not selected', () => {
		expect(toggleDocType(at('?doc_type_name=task'), 'goal')).toContain('task%2Cgoal');
	});

	it('removes a kind that is selected', () => {
		const out = toggleDocType(at('?doc_type_name=task,goal'), 'task');
		expect(out).toContain('doc_type_name=goal');
		expect(out).not.toContain('task');
	});

	it('drops the param entirely when the last kind is removed', () => {
		expect(toggleDocType(at('?doc_type_name=task'), 'task')).not.toContain('doc_type_name');
	});
});

describe('revealedKind', () => {
	const none = parseFilters(at(''));

	it('is the sole selected kind', () => {
		expect(revealedKind(parseFilters(at('?doc_type_name=task')), { task: 3, goal: 2 })).toBe('task');
	});

	it('is null when two kinds are selected', () => {
		expect(revealedKind(parseFilters(at('?doc_type_name=task,goal')), { task: 3, goal: 2 })).toBeNull();
	});

	// The arm that distinguishes the two rules: with no selection the histogram DOES
	// describe the fully filtered set, because excluding an absent predicate changes nothing.
	it('falls back to the histogram when nothing is selected', () => {
		expect(revealedKind(none, { task: 3 })).toBe('task');
	});

	it('is null when the histogram holds more than one kind and nothing is selected', () => {
		expect(revealedKind(none, { task: 3, goal: 2 })).toBeNull();
	});

	it('ignores zero-count histogram entries', () => {
		expect(revealedKind(none, { task: 3, goal: 0 })).toBe('task');
	});

	it('is null on an empty histogram', () => {
		expect(revealedKind(none, {})).toBeNull();
	});
});

describe('activeFilterCount', () => {
	it('is zero on an unfiltered URL', () => {
		expect(activeFilterCount(parseFilters(at('')))).toBe(0);
	});

	it('ignores sort, order and offset, which do not narrow', () => {
		expect(activeFilterCount(parseFilters(at('?sort=title&order=asc&offset=50')))).toBe(0);
	});

	it('counts a multi-value doc-type selection once', () => {
		expect(activeFilterCount(parseFilters(at('?doc_type_name=task,goal')))).toBe(1);
	});

	it('counts each distinct dimension', () => {
		expect(activeFilterCount(parseFilters(at('?doc_type_name=task&stage=done&q=atlas')))).toBe(3);
	});
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd packages/temper-ui && bun run test vault-filters`
Expected: FAIL — cannot resolve `./vault-filters`.

- [ ] **Step 3: Implement the module**

Write `vault-filters.ts` to satisfy the tests. Carry a module doc comment naming `nav.ts:158-181` as the pattern it mirrors and stating why `revealedKind` has two arms (the histogram stops describing the fully filtered set once a doc-type is selected — see spec D6).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-ui && bun run test vault-filters`
Expected: PASS, 16 tests.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/vault-filters.ts packages/temper-ui/src/lib/vault-filters.test.ts
git commit -m "feat(ui): pure URL filter state for the vault grid"
```

---

### Task 4: `vault-columns.ts` — kind-aware columns and the drift guard

**Files:**
- Create: `packages/temper-ui/src/lib/vault-columns.ts`
- Test: `packages/temper-ui/src/lib/vault-columns.test.ts`

**Interfaces:**
- Produces, relied on by Tasks 6 and 7:

```ts
export interface VaultColumn {
	id: string;
	header: string;
	width?: number;
	flexgrow?: number;
	sort: boolean;
	/** The `ResourceSortField` name to send, when it differs from `id`. */
	sortKey?: string;
}
export const KIND_KEYS: Readonly<Record<string, readonly string[]>>;
export function columnsFor(kind: string | null): VaultColumn[];
```

**Grounded facts:**
- Column objects must keep the shape `VaultGrid.svelte:34-40` already passes to `wx-svelte-grid` — `{ id, header, flexgrow|width, sort }`.
- Sortable ids are constrained by `ResourceSortField` (`generated/resource.ts:178`): `updated | created | title | stage | seq | context_name | doc_type_name`. **`status` is NOT sortable** — the goal's status column must be emitted with `sort: false`, or the header offers a sort the door cannot serve.
- **The column id and the sort field are different strings, deliberately.** The id is `temper-stage`, because Task 6 reads cell data as `r.managed_meta[id]`; the sort field the door accepts is `stage`. So `temper-stage` carries `sort: true` **and** `sortKey: 'stage'`, while `temper-status` carries `sort: false` and no `sortKey`. Sending `sort=temper-stage` would be rejected by the door as an invalid enum value — a header offering a sort that errors.
- Order defers to `MANAGED_KEY_ORDER` (`lib/properties.ts:21`).
- `KIND_KEYS` is `task → ['temper-stage']`, `goal → ['temper-status']`, everything else `→ []`. `temper-mode`/`temper-effort` are deliberately excluded (spec D7) — they are pre-work estimates revised during the work, so a column built on them ranks by a stale prediction.

- [ ] **Step 1: Write the failing tests**

The drift guard asserts **subset and exclusivity**, not equality — the map deliberately shows fewer keys than the schema declares, so an equality assertion would fail by design and would be the wrong guard.

```ts
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { MANAGED_KEY_ORDER } from './properties';
import { columnsFor, KIND_KEYS } from './vault-columns';

const schemaProps = (kind: string): string[] => {
	const path = new URL(
		`../../../../crates/temper-workflow/schemas/${kind}.schema.json`,
		import.meta.url,
	);
	return Object.keys(JSON.parse(readFileSync(path, 'utf8')).properties ?? {});
};

describe('KIND_KEYS drift guard', () => {
	it('names only keys the kind actually declares', () => {
		for (const [kind, keys] of Object.entries(KIND_KEYS)) {
			const declared = schemaProps(kind);
			for (const key of keys) {
				expect(declared, `${kind}.schema.json must declare ${key}`).toContain(key);
			}
		}
	});

	it('names only keys scoped to that kind and no other', () => {
		expect(schemaProps('goal')).not.toContain('temper-stage');
		expect(schemaProps('task')).not.toContain('temper-status');
	});

	it('uses keys that MANAGED_KEY_ORDER knows how to order', () => {
		for (const keys of Object.values(KIND_KEYS)) {
			for (const key of keys) {
				expect(MANAGED_KEY_ORDER).toContain(key);
			}
		}
	});
});

describe('columnsFor', () => {
	it('shows Type on a mixed set', () => {
		expect(columnsFor(null).map((c) => c.id)).toEqual([
			'title', 'context_name', 'doc_type_name', 'updated',
		]);
	});

	it('drops Type and reveals stage for an all-task set', () => {
		expect(columnsFor('task').map((c) => c.id)).toEqual([
			'title', 'context_name', 'temper-stage', 'updated',
		]);
	});

	it('drops Type and reveals status for an all-goal set', () => {
		expect(columnsFor('goal').map((c) => c.id)).toEqual([
			'title', 'context_name', 'temper-status', 'updated',
		]);
	});

	it('drops Type for a kind with no managed keys of its own', () => {
		expect(columnsFor('research').map((c) => c.id)).toEqual([
			'title', 'context_name', 'updated',
		]);
	});

	// status is absent from ResourceSortField, so offering a sort would overstate.
	it('marks the status column unsortable', () => {
		expect(columnsFor('goal').find((c) => c.id === 'temper-status')!.sort).toBe(false);
	});

	it('marks the stage column sortable', () => {
		expect(columnsFor('task').find((c) => c.id === 'temper-stage')!.sort).toBe(true);
	});

	// The id is the managed_meta key; the sort field the door accepts is `stage`.
	// Sending `sort=temper-stage` is rejected as an invalid ResourceSortField.
	it('carries a sortKey for the stage column, distinct from its id', () => {
		expect(columnsFor('task').find((c) => c.id === 'temper-stage')!.sortKey).toBe('stage');
	});

	it('gives no sortKey to columns whose id is already the sort field', () => {
		expect(columnsFor(null).find((c) => c.id === 'title')!.sortKey).toBeUndefined();
	});
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd packages/temper-ui && bun run test vault-columns`
Expected: FAIL — cannot resolve `./vault-columns`.

- [ ] **Step 3: Implement the module**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-ui && bun run test vault-columns`
Expected: PASS, 9 tests. If the drift guard cannot read the schema files, fix the relative path rather than deleting the guard — the path is the point.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/vault-columns.ts packages/temper-ui/src/lib/vault-columns.test.ts
git commit -m "feat(ui): kind-aware column derivation with a schema drift guard"
```

---

### Task 5: `FilterBar.svelte` and multi-select chips

**Files:**
- Create: `packages/temper-ui/src/lib/components/vault/FilterBar.svelte`
- Modify: `packages/temper-ui/src/lib/components/FacetChips.svelte`

**Interfaces:**
- Consumes: everything exported by `vault-filters.ts` (Task 3).
- Produces: `<FilterBar {filters} {revealed} {fixedContext} {contexts} />` for Task 7. **No `facets` prop** — `revealed` is already computed by `VaultBrowser` via `revealedKind`, so the bar never needs the histogram itself. (An earlier draft of this line declared one; it was dead on arrival and removed.)
- `contexts: ContextRowWithCounts[]` is the option list for the Context select. It is **not** a new fetch: `(app)/+layout.server.ts:37-48` already loads `/api/contexts` for the sidebar, so it arrives as layout data. Task 7 threads it through `VaultBrowser`. Each option's value is the decorated `` `${owner_ref}/${slug}` `` ref, which is exactly what the door's `context_ref` accepts (`resource.rs:66-69` — "UUID string or `@owner/slug` decorated ref. Bare context names are rejected server-side").

**Grounded facts:**
- Sibling to read first: `packages/temper-ui/src/lib/components/vault/PropertySet.svelte` and `HomeChip.svelte` — match their Svelte 5 runes style (`$props()`, `$derived`), Tailwind class usage, and the `quiet-*` colour tokens.
- `FacetChips.svelte:11` currently reads a single active value and `:21-30` toggles it. Rewrite both against `parseFilters().docTypes` and `toggleDocType`. Keep the existing chip markup and the count badge — only the selection model changes.
- The stage enum is `backlog | in-progress | done | cancelled`; the status enum is `active | completed | paused | cancelled`. Both come from `crates/temper-workflow/schemas/{task,goal}.schema.json`. **Do not cite `schema.rs:830/843` for these — those lines are test fixtures, not the source.**
- The `q` control is labelled **"title contains"**, never "search" — `substrate_read.rs` implements it as `r.title ILIKE '%' || $7 || '%'`, and mislabelling it is the live overstatement the goal names.

- [ ] **Step 1: Build the bar**

Render, left to right: the `q` input labelled "title contains"; a Context select (omitted entirely when `fixedContext` is true, because the route already fixes it); a tags token input; then — only when `revealed` is `'task'` or `'goal'` — the Stage or Status select over the enum above. A filter the visible kind cannot carry is **absent, never disabled** (spec D6).

Every control mutates the URL through `buildFilterUrl` and `goto(url, { replaceState: true })`, matching `VaultGrid.svelte:93`.

- [ ] **Step 2: Convert the chips to multi-select**

- [ ] **Step 3: Verify types and lint**

Run: `cd packages/temper-ui && bun run check && bun run biome`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/components/vault/FilterBar.svelte packages/temper-ui/src/lib/components/FacetChips.svelte
git commit -m "feat(ui): filter bar with procedurally revealed kind-scoped filters"
```

---

### Task 6: `VaultGrid` takes columns and reads the envelope

**Files:**
- Modify: `packages/temper-ui/src/lib/components/VaultGrid.svelte`

**Interfaces:**
- Consumes: `VaultColumn` from Task 4.
- Produces: `<VaultGrid {rows} {columns} {total} {returned} {truncated} {limit} {offset} />`.

**Grounded facts:**
- `VaultGrid.svelte:34-40` — replace the hardcoded const with a required `columns: VaultColumn[]` prop. Do not keep a default; a silent fallback would hide a mis-wired page.
- `VaultGrid.svelte:56-63` already maps `managed_meta['temper-stage']` onto a `stage` key. Generalise it: for each column id starting with `temper-`, read `r.managed_meta[id] ?? ''`.
- `VaultGrid.svelte:73` currently derives `hasNext` as `offset + limit < total`. Replace with the envelope's `truncated`, which the server derives as `offset + returned < total` (`resource.rs:208`) — deliberately *not* `total > returned`, which is true on the last page of a walk where nothing is in fact hidden.
- `SORTABLE` (`:18-26`) must be intersected with the active columns, so a header cannot offer a sort for a column that is not shown. `handleSort` (`:86-94`) must send `column.sortKey ?? column.id` — the revealed stage column's id is `temper-stage` but the door's sort field is `stage`, and `sort=temper-stage` is rejected as an invalid enum. `sortMarks` (`:43-50`) must map the URL's sort field back the same way, or the active-sort indicator lands on no column.
- The empty state at `:108-111` currently reads "No resources found." When any filter is active it must name what is narrowing and offer to clear — an unqualified "none" over a filtered set is the overstatement this arm exists to remove.

- [ ] **Step 1: Make the changes**

- [ ] **Step 2: Verify**

Run: `cd packages/temper-ui && bun run check && bun run biome && bun run test`
Expected: clean; existing tests still pass.

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/src/lib/components/VaultGrid.svelte
git commit -m "feat(ui): grid takes derived columns and reads the envelope's paging state"
```

---

### Task 7: `VaultBrowser` and the three pages

**Files:**
- Create: `packages/temper-ui/src/lib/components/vault/VaultBrowser.svelte`
- Modify: `packages/temper-ui/src/routes/(app)/vault/all/+page.{server.ts,svelte}`
- Modify: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.{server.ts,svelte}`
- Modify: `packages/temper-ui/src/routes/(app)/vault/search/+page.{server.ts,svelte}`

**Interfaces:**
- Consumes: Tasks 3–6.

**Grounded facts:**
- All three servers already forward `url.searchParams` wholesale (`vault/all/+page.server.ts:8-9` and siblings), so **filters need no server change to work** — verified safe because `ResourceListParams` carries no `deny_unknown_fields`, so UI-only params are ignored rather than 400ing.
- Each server must additionally return `returned` and `truncated`, and all three facet maps where today they return only `facets.doc_type` (`vault/all/+page.server.ts:20-22`). Each map is converted from the wire's `{ [key in string]?: bigint }` to `Record<string, number>` using the `Object.fromEntries(Object.entries(...).map(([k, v]) => [k, Number(v ?? 0)]))` idiom already at `:20-22` — the conversion is the incumbent, so extract it to a local helper rather than writing it three times per file.
- The context page fixes `context_ref` from the route (`[owner]/[context]/+page.server.ts:9`), so it mounts `VaultBrowser` with `fixedContext`.
- **`vault/search/+page.server.ts:21-32` must stop synthesizing a success envelope on a failed fetch.** Its own comment says a consumer reading `truncated` turns `truncated: false` into a lie, and Task 6 makes the grid exactly that consumer. Surface the error to the page and render an error state — do not pick a different boolean.
- The heading caption reads as a filtered count when any filter is active (`RuleHeading.svelte` takes `title` and `caption`; no change needed to that component).
- Do **not** touch the header control's "Search the vault…" copy or `_internal/search/+server.ts`. Real search is a separately sequenced task; this plan neither fixes nor worsens it.

- [ ] **Step 1: Build `VaultBrowser.svelte`**

Heading + `FilterBar` + `FacetChips` + `VaultGrid`, deriving `filters` via `parseFilters($page.url)`, `revealed` via `revealedKind(filters, facets.doc_type)`, and `columns` via `columnsFor(revealed)`.

`VaultBrowser` takes `contexts: ContextRowWithCounts[]` and passes it to `FilterBar`. Each page supplies it from the already-loaded layout data (`(app)/+layout.server.ts:37-48`) — do **not** add a `/api/contexts` fetch to any page server; the sidebar's copy is the incumbent and a second fetch would be a second source of truth for the same list.

- [ ] **Step 2: Collapse the three pages onto it**

- [ ] **Step 3: Fix the search page's error path**

- [ ] **Step 4: Verify**

Run: `cd packages/temper-ui && bun run check && bun run biome && bun run test`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src
git commit -m "feat(ui): one vault browser behind all three list routes"
```

---

## Final verification (controller)

- [ ] `cargo make check > /tmp/final.log 2>&1; tail -40 /tmp/final.log`
- [ ] `cargo test -p temper-services --features test-db --test list_facet_independence_test --test list_page_query_count_test > /tmp/final-db.log 2>&1; tail -30 /tmp/final-db.log`
- [ ] `cd packages/temper-ui && bun run check && bun run biome && bun run test`
- [ ] `git status` clean

**Declared limit — do not report this as verified.** There is no `/dev/vault` harness (only `/dev/atlas`), and Auth0 callbacks are prod-only, so authed routing cannot be browser-verified on a Vercel preview. The surface is verified by type-check, lint, and unit tests only; behavioural confirmation is post-merge in prod, as with prior beats.
