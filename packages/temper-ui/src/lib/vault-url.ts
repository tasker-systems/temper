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
