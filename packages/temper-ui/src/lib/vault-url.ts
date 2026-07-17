import type { ResourceRow } from '$lib/types/generated/resource';

/**
 * Single authority for `/vault/...` route URLs. Every nav link, back link,
 * row-click, and the Atlas rail's "View full resource" button routes through
 * these builders so addressing can never drift per call site.
 *
 * `ownerRef` is already sigil'd (`@<handle>` / `+<team-slug>`) and is NOT
 * percent-encoded — the sigils are valid path chars the `[owner]` route matches
 * literally. `slug` and `docType` are encoded defensively.
 */

export function contextHref(ownerRef: string, slug: string): string {
	return `/vault/${ownerRef}/${encodeURIComponent(slug)}`;
}

/**
 * The Atlas context door. Both the left-nav "Graph" link and the Home build
 * circle resolve here, so there is exactly one context-graph URL in the app.
 * `ownerRef` keeps its sigils (a valid path segment); the slug is the `?context`
 * scope and is percent-encoded.
 */
export function contextGraphHref(ownerRef: string, slug: string): string {
	return `/graph/${ownerRef}?context=${encodeURIComponent(slug)}`;
}

/** SvelteKit's `page.params` for the routes that can address a context. */
export interface ContextLocationParams {
	owner?: string;
	context?: string;
}

/**
 * Inverse of the two builders above: does `url` address `ownerRef`'s `slug`
 * context? `contextHref` carries the context as the `[context]` path segment;
 * `contextGraphHref` carries it as the `?context=` scope. Callers pass
 * `page.params` alongside `page.url` so the sigil'd `[owner]` segment is matched
 * by the router rather than re-parsed here.
 */
export function isContextLocation(
	params: ContextLocationParams,
	url: URL,
	ownerRef: string,
	slug: string
): boolean {
	if (params.owner !== ownerRef) return false;
	return (params.context ?? url.searchParams.get('context')) === slug;
}

/** True only on the Atlas door for `ownerRef`'s `slug` context. */
export function isContextGraphLocation(
	params: ContextLocationParams,
	url: URL,
	ownerRef: string,
	slug: string
): boolean {
	return isContextLocation(params, url, ownerRef, slug) && url.pathname.startsWith('/graph/');
}

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

export function searchHref(query: string): string {
	return `/vault/search?q=${encodeURIComponent(query)}`;
}
