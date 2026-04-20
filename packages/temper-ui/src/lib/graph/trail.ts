import type { GraphNode } from '../types/generated/graph';

/**
 * One rendered segment in the peek's breadcrumb bar.
 *
 *   - `kind: 'node'` — a clickable (or, when `isCurrent`, inert) crumb. `depth`
 *     is the *original* index in the untruncated trail, so click handlers
 *     can `trail.slice(0, depth + 1)` regardless of whether the trail was
 *     collapsed for rendering.
 *   - `kind: 'ellipsis'` — the `…` separator shown when a long trail is
 *     collapsed. Not clickable in this PR.
 */
export type CrumbEntry =
	| { kind: 'node'; node: GraphNode; depth: number; isCurrent: boolean }
	| { kind: 'ellipsis' };

/**
 * Trail length (in *resolved* crumbs) at which we collapse the middle of the
 * breadcrumb bar to `first › … › penult › current`. Matches the prototype.
 */
export const COLLAPSE_THRESHOLD = 5;

/**
 * Resolve trail ids to renderable crumbs, collapsing long trails.
 *
 * Rules:
 *   1. Drop trail ids that don't resolve against `nodes` (defensive — in
 *      practice the subgraph is static while the peek is open).
 *   2. If the resolved list is shorter than `COLLAPSE_THRESHOLD`, yield every
 *      entry with its original depth preserved.
 *   3. Otherwise yield `[first, ellipsis, penult, current]`.
 *   4. The last resolved entry is always marked `isCurrent: true` and is
 *      rendered as an inert span (no click).
 *
 * Callers treat `CrumbEntry.depth` as the argument to pass to
 * `onCrumbClick(depth)`; the component slices the trail to
 * `trail.slice(0, depth + 1)`.
 */
export function buildCrumbEntries(trail: string[], nodes: GraphNode[]): CrumbEntry[] {
	if (trail.length === 0) return [];
	const byId = new Map(nodes.map((n) => [n.id, n] as const));

	const resolved: { node: GraphNode; depth: number }[] = [];
	trail.forEach((id, i) => {
		const node = byId.get(id);
		if (node) resolved.push({ node, depth: i });
	});
	if (resolved.length === 0) return [];

	const last = resolved.length - 1;
	const mark = (i: number): CrumbEntry => ({
		kind: 'node',
		node: resolved[i].node,
		depth: resolved[i].depth,
		isCurrent: i === last
	});

	if (resolved.length < COLLAPSE_THRESHOLD) {
		return resolved.map((_, i) => mark(i));
	}
	// Collapsed: first, ellipsis, penult, current.
	return [mark(0), { kind: 'ellipsis' }, mark(last - 1), mark(last)];
}
