import type { GraphEdge, GraphNode } from '../types/generated/graph';

export interface Viewport {
	width: number;
	height: number;
}

export interface Position {
	x: number;
	y: number;
}

/**
 * Deterministically seed initial positions for all nodes.
 *
 * Concepts are laid out on an Archimedean spiral centered in the viewport.
 * Non-concept nodes are placed near their first linked concept (if any),
 * with a small deterministic jitter; orphaned non-concepts fall back to the
 * viewport center with their own jitter.
 *
 * Deterministic by construction — no Math.random().
 */
export function seedPositions(
	nodes: GraphNode[],
	edges: GraphEdge[],
	viewport: Viewport
): Map<string, Position> {
	const cx = viewport.width / 2;
	const cy = viewport.height / 2;
	const positions = new Map<string, Position>();

	// 1. Place concepts on a spiral.
	const concepts = nodes.filter((n) => n.doc_type === 'concept');
	const spiralA = 30;
	const spiralB = 20;
	concepts.forEach((node, i) => {
		const theta = i * 0.9;
		const r = spiralA + spiralB * theta;
		positions.set(node.id, {
			x: clamp(cx + r * Math.cos(theta), viewport.width),
			y: clamp(cy + r * Math.sin(theta), viewport.height)
		});
	});

	// 2. Build a quick "node id → first linked concept id" lookup.
	const conceptIds = new Set(concepts.map((c) => c.id));
	const firstConceptLink = new Map<string, string>();
	for (const edge of edges) {
		pickConceptLink(firstConceptLink, conceptIds, edge.source, edge.target);
		pickConceptLink(firstConceptLink, conceptIds, edge.target, edge.source);
	}

	// 3. Place non-concepts near their first linked concept with jitter.
	const nonConcepts = nodes.filter((n) => n.doc_type !== 'concept');
	nonConcepts.forEach((node, i) => {
		const anchor = firstConceptLink.get(node.id);
		const base = anchor ? positions.get(anchor) : undefined;
		const jitterX = deterministicJitter(i, 0) * 60;
		const jitterY = deterministicJitter(i, 1) * 60;
		positions.set(node.id, {
			x: clamp((base?.x ?? cx) + jitterX, viewport.width),
			y: clamp((base?.y ?? cy) + jitterY, viewport.height)
		});
	});

	return positions;
}

function pickConceptLink(
	into: Map<string, string>,
	conceptIds: Set<string>,
	key: string,
	otherEnd: string
): void {
	if (into.has(key)) return;
	if (conceptIds.has(key)) return; // the node IS a concept
	if (conceptIds.has(otherEnd)) into.set(key, otherEnd);
}

function clamp(v: number, limit: number): number {
	return Math.max(0, Math.min(limit, v));
}

function deterministicJitter(i: number, axis: number): number {
	// Cheap, deterministic jitter in the range roughly (-1, 1).
	// Uses sine of the index so successive nodes fan out.
	const seed = i * 12.9898 + axis * 78.233;
	const s = Math.sin(seed);
	return s - Math.floor(s) - 0.5;
}
