// packTerritories.ts
/**
 * Tier-0 / Tier-1 cartographic layout: pack territories into circles sized by
 * member_count using d3-hierarchy. Pure — takes data + a box, returns positions.
 * No force simulation (force runs only on Tier-2 neighborhoods, per spec D1).
 */
import { hierarchy, pack } from 'd3-hierarchy';
import type { Territory } from '$lib/types/generated/graph_territory';

export interface PositionedTerritory {
	id: string;
	kind: Territory['kind'];
	label: string | null;
	anchorId: string;
	x: number;
	y: number;
	r: number;
	salience: number | null;
}

interface PackDatum {
	territory?: Territory;
	children?: PackDatum[];
}

export function packTerritories(
	territories: Territory[],
	size: { width: number; height: number }
): PositionedTerritory[] {
	if (territories.length === 0) return [];

	const root = hierarchy<PackDatum>({
		children: territories.map((t) => ({ territory: t }))
	})
		.sum((d) => {
			if (!d.territory) return 0;
			const t = d.territory;
			return t.kind === 'region'
				? Math.max(1, Math.round((t.salience ?? 0) * 100))
				: Math.max(1, t.member_count);
		});

	const layout = pack<PackDatum>().size([size.width, size.height]).padding(6);
	const packed = layout(root);

	return packed.leaves().map((leaf) => {
		const t = leaf.data.territory!;
		return {
			id: t.id,
			kind: t.kind,
			label: t.label,
			anchorId: t.anchor_id,
			x: leaf.x,
			y: leaf.y,
			r: leaf.r,
			salience: t.salience
		};
	});
}
