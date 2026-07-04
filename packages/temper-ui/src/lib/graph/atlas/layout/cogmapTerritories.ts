/**
 * Sparse-state layout (spec C2-D4): orphan nodes carry the cogmap they're homed
 * in (anchor_id). Group them into synthetic cogmap territories and pack each
 * cogmap's facets INSIDE its hull — the same cartographic language as dense
 * territories, but for region-less cogmaps. Pure; no force (Tiers 0/1 pack).
 */
import { hierarchy, pack } from 'd3-hierarchy';
import type { OrphanNode } from '$lib/types/generated/graph_territory';

export interface PositionedFacet {
	id: string;
	title: string;
	docType: string | null;
	x: number;
	y: number;
	r: number;
}

export interface CogmapTerritory {
	cogmapId: string;
	label: string;
	facetCount: number;
	x: number;
	y: number;
	r: number;
	facets: PositionedFacet[];
}

interface Node {
	orphan?: OrphanNode;
	cogmapId?: string;
	children?: Node[];
}

export function packCogmapTerritories(
	orphans: OrphanNode[],
	size: { width: number; height: number }
): CogmapTerritory[] {
	if (orphans.length === 0) return [];

	// group by cogmap (anchor_id), stable insertion order
	const groups = new Map<string, OrphanNode[]>();
	for (const o of orphans) {
		const g = groups.get(o.anchor_id);
		if (g) g.push(o);
		else groups.set(o.anchor_id, [o]);
	}

	// outer pack: one leaf per cogmap, sized by facet count
	const root = hierarchy<Node>({
		children: [...groups.entries()].map(([cogmapId, facets]) => ({
			cogmapId,
			children: facets.map((orphan) => ({ orphan }))
		}))
	}).sum((d) => (d.orphan ? 1 : 0));

	const packed = pack<Node>().size([size.width, size.height]).padding(14)(root);

	return (packed.children ?? []).map((group) => {
		const cogmapId = group.data.cogmapId!;
		const facets: PositionedFacet[] = (group.children ?? []).map((leaf) => ({
			id: leaf.data.orphan!.id,
			title: leaf.data.orphan!.title,
			docType: leaf.data.orphan!.doc_type,
			x: leaf.x,
			y: leaf.y,
			r: Math.max(4, leaf.r)
		}));
		// All facets in a group share anchor_id, so any element's anchor_label works.
		const anchorLabel = group.children?.[0]?.data.orphan?.anchor_label;
		return {
			cogmapId,
			label: `${anchorLabel ?? 'cogmap'} · ${facets.length} facets`,
			facetCount: facets.length,
			x: group.x,
			y: group.y,
			r: group.r,
			facets
		};
	});
}
