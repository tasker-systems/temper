// nav.ts
/**
 * Atlas navigation is expressed entirely in the URL — the "URI frame". The route
 * `/vault/[owner]/graph` carries `?team`, `?focus`, and filter params; the tier is
 * DERIVED from focus, never stored. Zoom/pan is ephemeral client state and is NOT
 * in the URL. See spec D2. Builders return a relative `path?query` string ready to
 * hand to `goto()`.
 *
 * History mode is chosen at the call site, not here:
 *   - Scope/drill transitions (buildCogmapUrl, buildDrillTerritoryUrl,
 *     buildDrillNodeUrl, buildAscendUrl, buildHomeUrl) PUSH history so the browser
 *     Back button walks the drill path (Atlas ← cogmap ← territory ← node).
 *   - Ephemeral view state (buildFiltersUrl, buildEdgeSelectUrl, clearSelectionUrl)
 *     REPLACES history so filter toggles and panel selection don't clutter the path.
 */
export type Tier = 0 | 1 | 2;

export type Focus =
	| { kind: 'none' }
	| { kind: 'territory'; id: string }
	| { kind: 'node'; id: string };

export interface GraphFilters {
	/** Optional lens id driving Tier-0 salience sizing (R2 `?lens_id`). */
	lensId: string | null;
	/** `?edge_kinds` CSV — restrict rendered edges to these kinds (ScopeBar, Task 8). */
	edgeKinds: string[];
	/** `?doc_types` CSV — restrict rendered nodes to these doc types (ScopeBar, Task 8). */
	docTypes: string[];
}

/** `?cogmap` addressing — entering a cogmap door is a distinct scope from a team (spec Task 5). */
export function parseCogmap(url: URL): string | null {
	return url.searchParams.get('cogmap');
}

/** Parse one `kind:id` focus token; null if malformed. */
function parseFocusToken(raw: string): Focus | null {
	const [kind, id] = raw.split(':', 2);
	if (id && (kind === 'territory' || kind === 'node')) return { kind, id };
	return null;
}

/** The full drill path (`?focus=territory:X,node:Y` → [territory, node]). */
export function parseFocusPath(url: URL): Focus[] {
	const raw = url.searchParams.get('focus');
	if (!raw) return [];
	return raw.split(',').map(parseFocusToken).filter((f): f is Focus => f !== null);
}

/** The active focus = the leaf (last) segment; drives tier + neighborhood seeding. */
export function parseFocus(params: URLSearchParams): Focus {
	const raw = params.get('focus');
	if (!raw) return { kind: 'none' };
	const segs = raw.split(',').map(parseFocusToken).filter((f): f is Focus => f !== null);
	return segs.length ? segs[segs.length - 1] : { kind: 'none' };
}

export type Selection =
	| { kind: 'none' }
	| { kind: 'edge'; id: string };

/** Orthogonal panel selection for edges. `?focus` still owns scope/camera/seed;
 *  `?sel=edge:<id>` selects an edge for the TrailRail panel without re-seeding. */
export function parseSelection(url: URL): Selection {
	const raw = url.searchParams.get('sel');
	if (!raw) return { kind: 'none' };
	const [kind, id] = raw.split(':', 2);
	if (id && kind === 'edge') return { kind: 'edge', id };
	return { kind: 'none' };
}

export function buildEdgeSelectUrl(base: URL, edgeId: string): string {
	return withParams(base, (p) => p.set('sel', `edge:${edgeId}`));
}

export function clearSelectionUrl(base: URL): string {
	return withParams(base, (p) => p.delete('sel'));
}

/** The element whose detail panel is shown: an explicitly-selected edge wins,
 *  else the focused node, else nothing. */
export type SelectedElement =
	| { kind: 'none' }
	| { kind: 'node'; id: string }
	| { kind: 'edge'; id: string };

export function selectedElement(focus: Focus, url: URL): SelectedElement {
	const sel = parseSelection(url);
	if (sel.kind === 'edge') return sel;
	if (focus.kind === 'node') return { kind: 'node', id: focus.id };
	return { kind: 'none' };
}

export function deriveTier(focus: Focus): Tier {
	switch (focus.kind) {
		case 'territory':
			return 1;
		case 'node':
			return 2;
		default:
			return 0;
	}
}

export function parseFilters(params: URLSearchParams): GraphFilters {
	const csv = (k: string) => {
		const v = params.get(k);
		return v ? v.split(',').filter(Boolean) : [];
	};
	return { lensId: params.get('lens_id'), edgeKinds: csv('edge_kinds'), docTypes: csv('doc_types') };
}

/** Count of active (non-default) filter dimensions — drives the popover badge. */
export function activeFilterCount(url: URL): number {
	const f = parseFilters(url.searchParams);
	return (f.lensId ? 1 : 0) + (f.edgeKinds.length ? 1 : 0) + (f.docTypes.length ? 1 : 0);
}

export function buildFiltersUrl(
	base: URL,
	patch: Partial<{ lensId: string | null; edgeKinds: string[]; docTypes: string[] }>
): string {
	return withParams(base, (p) => {
		if ('lensId' in patch) {
			if (patch.lensId) p.set('lens_id', patch.lensId);
			else p.delete('lens_id');
		}
		const setCsv = (k: string, v?: string[]) => {
			if (!v) return;
			if (v.length) p.set(k, v.join(','));
			else p.delete(k);
		};
		setCsv('edge_kinds', patch.edgeKinds);
		setCsv('doc_types', patch.docTypes);
	});
}

function withParams(base: URL, mutate: (p: URLSearchParams) => void): string {
	const u = new URL(base);
	mutate(u.searchParams);
	return `${u.pathname}${u.search}`;
}

/** Enter a cogmap door: set cogmap, clear focus (re-scope resets to Tier 0). */
export function buildCogmapUrl(base: URL, cogmapId: string): string {
	return withParams(base, (p) => {
		p.set('cogmap', cogmapId);
		p.delete('focus');
	});
}

export function buildDrillTerritoryUrl(base: URL, territoryId: string): string {
	// A territory is always the first drill hop from the panorama.
	return withParams(base, (p) => p.set('focus', `territory:${territoryId}`));
}

export function buildDrillNodeUrl(base: URL, nodeId: string): string {
	// Replace a trailing node leaf (node→node drill) while KEEPING any preceding
	// territory prefix; otherwise append to a territory leaf, or set directly
	// from the panorama (no focus yet).
	return withParams(base, (p) => {
		const path = (p.get('focus') ?? '').split(',').filter(Boolean);
		if (path[path.length - 1]?.startsWith('node:')) path.pop();
		path.push(`node:${nodeId}`);
		p.set('focus', path.join(','));
	});
}

export function buildAscendUrl(base: URL): string {
	// Pop exactly one drill level (node → its territory → panorama).
	return withParams(base, (p) => {
		const path = (p.get('focus') ?? '').split(',').filter(Boolean);
		path.pop();
		if (path.length) p.set('focus', path.join(','));
		else p.delete('focus');
	});
}

/** The Atlas Home lens (Beat B). Neutral (rest) is the absence of `?home`. */
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

/** Return to the neutral Home selection (drop the committed lens). */
export function clearHomeLensUrl(base: URL): string {
	return withParams(base, (p) => p.delete('home'));
}

/** Return to the membership home: clear team, cogmap, and focus. */
export function buildHomeUrl(base: URL): string {
	return withParams(base, (p) => {
		p.delete('team');
		p.delete('cogmap');
		p.delete('focus');
	});
}

/** Drop back to the panorama of the current scope: clear `focus` (→ Tier 0) and any
 *  edge selection, but keep the active team/cogmap and filters. Used to degrade
 *  gracefully when a focused territory has been re-materialized out from under the URL
 *  (its region id now 404s) — the cluster is gone, so land the user on the current map. */
export function buildPanoramaUrl(base: URL): string {
	return withParams(base, (p) => {
		p.delete('focus');
		p.delete('sel');
	});
}
