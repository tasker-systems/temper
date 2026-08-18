// vault-filters.ts
/**
 * The vault grid's filter state lives entirely in the URL, mirroring the Atlas pattern
 * at `graph/atlas/nav.ts:158-181`: a private `withParams(base, mutate)` helper returns
 * `${pathname}${search}`, multi-value params round-trip as CSV, and an empty/null value
 * is deleted rather than written as `''`. `nav.ts` keeps its `withParams` private and is
 * Atlas-scoped, so it is duplicated here rather than imported.
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
		stage: p.get('stage'),
		status: p.get('status'),
		contextRef: p.get('context_ref'),
		q: p.get('q'),
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

export function toggleDocType(base: URL, name: string): string {
	const current = parseFilters(base).docTypes;
	const next = current.includes(name) ? current.filter((n) => n !== name) : [...current, name];
	return buildFilterUrl(base, { docTypes: next });
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
