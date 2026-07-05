import type { Bridge } from '$lib/types/generated/graph_territory';

export interface BridgeLine {
	x1: number;
	y1: number;
	x2: number;
	y2: number;
	edgeCount: number;
}

/**
 * Map aggregate bridges to line segments between territory centers. Position-
 * agnostic: pass any `Map<territoryId, {x,y}>` — packing today, force later.
 * Bridges with an unpositioned endpoint are dropped.
 */
export function bridgeGeometry(
	bridges: Bridge[],
	positions: Map<string, { x: number; y: number }>
): BridgeLine[] {
	const lines: BridgeLine[] = [];
	for (const b of bridges) {
		const s = positions.get(b.source_territory);
		const t = positions.get(b.target_territory);
		if (!s || !t) continue;
		lines.push({ x1: s.x, y1: s.y, x2: t.x, y2: t.y, edgeCount: b.edge_count });
	}
	return lines;
}
