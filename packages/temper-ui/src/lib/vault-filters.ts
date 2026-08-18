// vault-filters.ts
/**
 * The vault grid's filter state lives entirely in the URL, mirroring the Atlas pattern
 * at `graph/atlas/nav.ts:158-181`: a private `withParams(base, mutate)` helper returns
 * `${pathname}${search}`, multi-value params round-trip as CSV, and an empty/null value
 * is deleted rather than written as `''`. `nav.ts` keeps its `withParams` private and is
 * Atlas-scoped, so it is duplicated here rather than imported.
 *
 * **Empty means absent, for every filter.** A param whose value is empty or only
 * whitespace (`?stage=`, `?q=%20`, `?doc_type_name=,,`) parses as though the param were
 * not there at all. This is one uniform rule rather than a per-filter accident: before it,
 * `?stage=` parsed as the empty string, which `activeFilterCount` counted as zero while
 * the door still applied it — so the grid rendered "No resources found." with no
 * Clear-filters affordance, for a filter the UI did not believe existed. The CSV filters
 * already behaved this way; the scalars now match them.
 *
 * Every mutation deletes `?offset` — narrowing the filter set always resets to page one
 * (matches `FacetChips.svelte:28` and `VaultGrid.svelte:92`).
 *
 * `revealedKind` has two arms because the `doc_type` facet histogram was changed (spec D6)
 * to EXCLUDE its own predicate: it counts what the OTHER filters admit, not what doc-type
 * selection itself admits. So:
 *   - When a doc-type IS selected, the histogram no longer describes the fully filtered
 *     set (it describes a strictly larger, pre-doc-type-filter set) — the selection itself
 *     is authoritative, and reveals only when exactly one kind is selected.
 *   - When NO doc-type is selected, excluding an absent predicate changes nothing, so the
 *     histogram DOES describe the fully filtered set — it reveals only when the histogram's
 *     non-zero entries name exactly one kind.
 */

export interface VaultFilters {
	docTypes: string[];
	stage: string | null;
	status: string | null;
	contextRef: string | null;
	q: string | null;
	tags: string[];
}

/**
 * The stage/status vocabularies the two kind-scoped selects offer.
 *
 * Enum sources: `crates/temper-workflow/schemas/task.schema.json` (`temper-stage`) and
 * `goal.schema.json` (`temper-status`). `schema.rs:830/843` carries matching literals but
 * is test-fixture code, not the source — never cite it for these values.
 *
 * These live here, beside a `.test.ts`, rather than inline in `FilterBar.svelte`, so the
 * drift guard in `vault-filters.test.ts` can read the schema JSON off disk and check them
 * — the same guard `KIND_KEYS` gets in `vault-columns.test.ts`. A hand-copied enum with no
 * guard is exactly how the select comes to offer a value the schema no longer declares.
 */
export const STAGES: readonly string[] = ['backlog', 'in-progress', 'done', 'cancelled'];
export const STATUSES: readonly string[] = ['active', 'completed', 'paused', 'cancelled'];

/**
 * Which kind each kind-scoped filter belongs to. `stage` is a `task` key and `status` is a
 * `goal` key (`crates/temper-workflow/schemas/`), and `FilterBar` reveals each select only
 * when that kind is the revealed one — so a selection that stops revealing the kind also
 * takes away the only control that can clear the filter.
 */
const KIND_SCOPED_FILTERS: Readonly<Record<'stage' | 'status', string>> = {
	stage: 'task',
	status: 'goal',
};

/**
 * Trim to `null`. Empty and whitespace-only are the same as absent — see the uniform rule
 * in the module doc above.
 */
function scalar(params: URLSearchParams, key: string): string | null {
	const v = params.get(key);
	if (v === null) return null;
	const trimmed = v.trim();
	return trimmed === '' ? null : trimmed;
}

function csv(params: URLSearchParams, key: string): string[] {
	const v = params.get(key);
	if (!v) return [];
	return v
		.split(',')
		.map((s) => s.trim())
		.filter(Boolean);
}

export function parseFilters(url: URL): VaultFilters {
	const p = url.searchParams;
	return {
		docTypes: csv(p, 'doc_type_name'),
		stage: scalar(p, 'stage'),
		status: scalar(p, 'status'),
		contextRef: scalar(p, 'context_ref'),
		q: scalar(p, 'q'),
		tags: csv(p, 'tags'),
	};
}

function withParams(base: URL, mutate: (p: URLSearchParams) => void): string {
	const u = new URL(base);
	mutate(u.searchParams);
	return `${u.pathname}${u.search}`;
}

export function buildFilterUrl(base: URL, patch: Partial<VaultFilters>): string {
	return withParams(base, (p) => {
		const setCsv = (key: string, v?: string[]) => {
			if (!v) return;
			if (v.length) p.set(key, v.join(','));
			else p.delete(key);
		};
		const setScalar = (key: string, v?: string | null) => {
			if (v === undefined) return;
			if (v) p.set(key, v);
			else p.delete(key);
		};
		if ('docTypes' in patch) setCsv('doc_type_name', patch.docTypes);
		if ('stage' in patch) setScalar('stage', patch.stage);
		if ('status' in patch) setScalar('status', patch.status);
		if ('contextRef' in patch) setScalar('context_ref', patch.contextRef);
		if ('q' in patch) setScalar('q', patch.q);
		if ('tags' in patch) setCsv('tags', patch.tags);
		p.delete('offset');
	});
}

/**
 * The kind-scoped filters a doc-type selection has taken the control away from, as a patch
 * that clears them.
 *
 * `FilterBar` renders the Stage select only when `task` is the revealed kind and the Status
 * select only when `goal` is. A selection of two or more kinds reveals nothing, and a
 * selection of one other kind reveals that one — so `?doc_type_name=task,goal&stage=done`
 * leaves `stage` applied by the door with no control on screen to clear it and, because rows
 * still come back, no "Clear filters" link either. Clearing it in the same URL mutation is
 * the only way the selection change cannot strand a filter.
 *
 * An EMPTY selection is deliberately left alone. With no doc-type selected `revealedKind`
 * falls back to the histogram, and the histogram IS scoped by stage/status — so an active
 * `stage` collapses it to the kinds that carry a stage (today: `task` alone), the select
 * stays on screen, and the filter stays clearable. Clearing on deselect would instead
 * discard a filter the user can still see and still use.
 */
export function kindScopedClears(nextDocTypes: string[]): Partial<VaultFilters> {
	if (nextDocTypes.length === 0) return {};
	const revealed = nextDocTypes.length === 1 ? nextDocTypes[0] : null;
	const patch: Partial<VaultFilters> = {};
	for (const [filter, kind] of Object.entries(KIND_SCOPED_FILTERS)) {
		if (kind !== revealed) patch[filter as 'stage' | 'status'] = null;
	}
	return patch;
}

export function toggleDocType(base: URL, name: string): string {
	const current = parseFilters(base).docTypes;
	const next = current.includes(name) ? current.filter((n) => n !== name) : [...current, name];
	return buildFilterUrl(base, { docTypes: next, ...kindScopedClears(next) });
}

/** One doc-type facet chip: its name, its count, and whether it is currently selected. */
export interface DocTypeChip {
	name: string;
	count: number;
	active: boolean;
}

/**
 * The doc-type chips to render, ordered by count descending then name.
 *
 * A SELECTED kind always gets a chip, even when the histogram has no entry for it. The door
 * emits no zero-count keys, so a selection that admits nothing (`?doc_type_name=blueprint`
 * over a vault with none, or a stage filter that empties the kind) would otherwise render no
 * chip at all — and the chip is the only control that can deselect it. Rendering it at 0 is
 * both the honest count and the way back out.
 */
export function docTypeChips(
	facets: Record<string, number> | null,
	selected: string[],
): DocTypeChip[] {
	const counts = new Map<string, number>(Object.entries(facets ?? {}));
	for (const name of selected) {
		if (!counts.has(name)) counts.set(name, 0);
	}
	return [...counts.entries()]
		.map(([name, count]) => ({ name, count, active: selected.includes(name) }))
		.sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

export function revealedKind(filters: VaultFilters, facets: Record<string, number>): string | null {
	if (filters.docTypes.length > 0) {
		return filters.docTypes.length === 1 ? filters.docTypes[0] : null;
	}
	const nonZero = Object.entries(facets).filter(([, count]) => count > 0);
	return nonZero.length === 1 ? nonZero[0][0] : null;
}

export function activeFilterCount(filters: VaultFilters): number {
	let count = 0;
	if (filters.docTypes.length > 0) count += 1;
	if (filters.stage) count += 1;
	if (filters.status) count += 1;
	if (filters.contextRef) count += 1;
	if (filters.q) count += 1;
	if (filters.tags.length > 0) count += 1;
	return count;
}
