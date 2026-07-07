import { forceCenter, forceCollide, forceManyBody, forceSimulation, forceX, forceY, type SimulationNodeDatum } from 'd3-force';
import type { Territory } from '$lib/types/generated/graph_territory';
import type { PositionedTerritory } from './packTerritories';

const TICKS = 300;
const LABEL_BAND = 26;
const R_MIN = 11;
const R_MAX = 42;

interface SimTerritory extends SimulationNodeDatum, PositionedTerritory {}

function territoryRadius(t: Territory, maxWeight: number): number {
	const weight = t.kind === 'region' ? (t.salience ?? 0) : Math.max(1, t.member_count);
	const norm = maxWeight > 0 ? Math.sqrt(weight / maxWeight) : 0;
	return R_MIN + norm * (R_MAX - R_MIN);
}

export function forceTerritories(
	territories: Territory[],
	size: { width: number; height: number }
): PositionedTerritory[] {
	if (territories.length === 0) return [];
	const maxWeight = Math.max(
		...territories.map((t) => (t.kind === 'region' ? (t.salience ?? 0) : Math.max(1, t.member_count)))
	);
	const n = territories.length;
	const cx = size.width / 2;
	const cy = size.height / 2;
	const spread = Math.min(size.width, size.height) * 0.42;
	const nodes: SimTerritory[] = territories.map((t, i) => ({
		id: t.id,
		kind: t.kind,
		label: t.label,
		anchorId: t.anchor_id,
		salience: t.salience,
		member_count: t.member_count,
		r: territoryRadius(t, maxWeight),
		x: cx + Math.cos((i / Math.max(1, n)) * 2 * Math.PI) * spread,
		y: cy + Math.sin((i / Math.max(1, n)) * 2 * Math.PI) * spread
	}));
	const sim = forceSimulation(nodes)
		.force('charge', forceManyBody().strength(-40))
		.force('center', forceCenter(cx, cy))
		.force('x', forceX(cx).strength(0.04))
		.force('y', forceY(cy).strength(0.06))
		.force('collide', forceCollide<SimTerritory>().radius((d) => d.r + LABEL_BAND / 2 + 3))
		.stop();
	for (let i = 0; i < TICKS; i++) sim.tick();
	return nodes.map((d) => ({
		id: d.id,
		kind: d.kind,
		label: d.label,
		anchorId: d.anchorId,
		x: d.x,
		y: d.y,
		r: d.r,
		salience: d.salience,
		member_count: d.member_count
	}));
}
