// nav.ts
/**
 * Atlas navigation is expressed entirely in the URL — the "URI frame". The route
 * `/vault/[owner]/graph` carries `?team`, `?focus`, and filter params; the tier is
 * DERIVED from focus, never stored. Zoom/pan is ephemeral client state and is NOT
 * in the URL. See spec D2. Builders return a relative `path?query` string ready to
 * hand to `goto(…, { replaceState: true })`.
 */
export type Tier = 0 | 1 | 2;

export type Focus =
	| { kind: 'none' }
	| { kind: 'territory'; id: string }
	| { kind: 'node'; id: string };

export interface GraphFilters {
	/** Optional lens id driving Tier-0 salience sizing (R2 `?lens_id`). */
	lensId: string | null;
}

export function parseTeam(params: URLSearchParams): string | null {
	return params.get('team');
}

export function parseFocus(params: URLSearchParams): Focus {
	const raw = params.get('focus');
	if (!raw) return { kind: 'none' };
	const [kind, id] = raw.split(':', 2);
	if (id && (kind === 'territory' || kind === 'node')) return { kind, id };
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
	return { lensId: params.get('lens_id') };
}

function withParams(base: URL, mutate: (p: URLSearchParams) => void): string {
	const u = new URL(base);
	mutate(u.searchParams);
	return `${u.pathname}${u.search}`;
}

/** Enter a team zone / switch team: set team, clear focus (re-scope resets to Tier 0). */
export function buildScopeUrl(base: URL, teamId: string): string {
	return withParams(base, (p) => {
		p.set('team', teamId);
		p.delete('focus');
	});
}

export function buildDrillTerritoryUrl(base: URL, territoryId: string): string {
	return withParams(base, (p) => p.set('focus', `territory:${territoryId}`));
}

export function buildDrillNodeUrl(base: URL, nodeId: string): string {
	return withParams(base, (p) => p.set('focus', `node:${nodeId}`));
}

export function buildAscendUrl(base: URL): string {
	return withParams(base, (p) => p.delete('focus'));
}

/** Return to the membership home: clear both team and focus. */
export function buildHomeUrl(base: URL): string {
	return withParams(base, (p) => {
		p.delete('team');
		p.delete('focus');
	});
}
