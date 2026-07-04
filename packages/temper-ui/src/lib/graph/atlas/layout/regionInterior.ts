/**
 * Tier-1 region interior layout (spec C2-D5): pack the region's members by
 * affinity inside the hull. Members are the payload; components are surfaced as
 * a badge by the component, not spatially (R3 gives no member→component map).
 * Pure; no force.
 */
import { hierarchy, pack } from 'd3-hierarchy';
import type { RegionMember } from '$lib/types/generated/graph_territory';

export interface PositionedMember {
	id: string;
	title: string;
	docType: string | null;
	affinity: number | null;
	x: number;
	y: number;
	r: number;
}

interface Node {
	member?: RegionMember;
	children?: Node[];
}

export function packRegionMembers(
	members: RegionMember[],
	size: { width: number; height: number }
): PositionedMember[] {
	if (members.length === 0) return [];

	const root = hierarchy<Node>({
		children: members.map((member) => ({ member }))
	}).sum((d) => (d.member ? Math.max(1, Math.round((d.member.affinity ?? 0) * 100)) : 0));

	const packed = pack<Node>().size([size.width, size.height]).padding(8)(root);

	return packed.leaves().map((leaf) => {
		const m = leaf.data.member!;
		return {
			id: m.id,
			title: m.title,
			docType: m.doc_type,
			affinity: m.affinity,
			x: leaf.x,
			y: leaf.y,
			r: Math.max(4, leaf.r)
		};
	});
}
