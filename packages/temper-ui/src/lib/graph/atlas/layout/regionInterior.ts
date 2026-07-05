/**
 * Tier-1 region interior layout (spec C2-D5): pack the region's members by
 * affinity inside the hull. Members are the payload; components are surfaced as
 * a badge by the component, not spatially (R3 gives no member→component map).
 * Pure; no force.
 *
 * Circles are packed at BOUNDED radii (`packSiblings`), not scaled to fill the
 * viewport (`pack().size()`). The fill-to-box approach inflated a 2–3 member
 * region into canvas-filling, overlapping blobs with colliding centre labels
 * (Beat-2a legibility regression); here a member's radius is derived from a
 * comfortable target size for the member count (nudged by affinity) and clamped,
 * then the packed cluster is centred and only ever scaled DOWN if it would
 * exceed the box.
 */
import { packEnclose, packSiblings } from 'd3-hierarchy';
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

/** Legible bounds for an interior member chip. */
const MIN_R = 14;
const MAX_R = 58;
/** Fraction of the box area the packed circles aim to cover before clamping. */
const FILL_FRACTION = 0.55;
/** Breathing room between the packed cluster and the box edge. */
const EDGE_PAD = 16;

interface PackCircle {
	member: RegionMember;
	r: number;
	x: number;
	y: number;
}

export function packRegionMembers(
	members: RegionMember[],
	size: { width: number; height: number }
): PositionedMember[] {
	if (members.length === 0) return [];

	// Target radius so N equal circles cover ~FILL_FRACTION of the box, clamped to
	// legible bounds — so a 3-member region gets prominent (not blob-sized) chips
	// and a crowded region gets small ones, both without overlap.
	const area = Math.max(1, size.width * size.height);
	const target = Math.sqrt((area * FILL_FRACTION) / (members.length * Math.PI));
	const baseR = Math.min(MAX_R, Math.max(MIN_R, target));

	const circles: PackCircle[] = members.map((member) => {
		// Affinity (when present) modulates the radius monotonically around the base
		// and stays clamped to [MIN_R, MAX_R]; absent affinity (the common case in
		// real data) leaves every chip at the neutral base size rather than flooring
		// it, so an all-null region reads as legible chips, not lost dots.
		const a = member.affinity == null ? null : Math.min(1, Math.max(0, member.affinity));
		const r = a == null ? baseR : Math.min(MAX_R, Math.max(MIN_R, baseR * (0.65 + 0.7 * a)));
		return { member, r, x: 0, y: 0 };
	});

	packSiblings(circles); // assigns x/y so circles touch without overlapping
	const enclose = packEnclose(circles.map((c) => ({ x: c.x, y: c.y, r: c.r })));

	// Never enlarge (would re-inflate a sparse region); only shrink to fit the box.
	const maxRadius = Math.max(1, Math.min(size.width, size.height) / 2 - EDGE_PAD);
	const scale = enclose.r > maxRadius ? maxRadius / enclose.r : 1;
	const cx = size.width / 2;
	const cy = size.height / 2;

	return circles.map((c) => ({
		id: c.member.id,
		title: c.member.title,
		docType: c.member.doc_type,
		affinity: c.member.affinity,
		x: cx + (c.x - enclose.x) * scale,
		y: cy + (c.y - enclose.y) * scale,
		r: Math.max(4, c.r * scale)
	}));
}
