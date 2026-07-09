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
 * Full-resource path for a context-homed resource. Returns `null` for a
 * cogmap-homed resource (its `context_*` fields are null) so callers can gate
 * the affordance rather than emit a broken URL. The final segment is the bare
 * `id`: the route resolves trailing-UUID-only, and `ResourceRow` carries no
 * resource slug to decorate with.
 */
export function resourceHref(row: ResourceRow): string | null {
	if (!row.context_owner_ref || !row.context_slug) return null;
	return `/vault/${row.context_owner_ref}/${encodeURIComponent(row.context_slug)}/${encodeURIComponent(row.doc_type_name)}/${row.id}`;
}

export function searchHref(query: string): string {
	return `/vault/search?q=${encodeURIComponent(query)}`;
}
