import type { GraphNode } from '../types/generated/graph';
import { decoratedRef } from '../ref';

/** Build the canonical vault-path href for a node. */
export function resourceHref(owner: string, context: string, node: GraphNode): string {
	// `@me` must not be encoded — `encodeURIComponent` would escape `@` to `%40`
	// and break the route. Owner handles are URL-safe by construction.
	return `/vault/${owner}/${encodeURIComponent(context)}/${encodeURIComponent(node.doc_type)}/${encodeURIComponent(decoratedRef(node.slug, node.id))}`;
}
