import { describe, expect, it } from 'vitest';
import { packRegionMembers } from './regionInterior';
import type { RegionMember } from '$lib/types/generated/graph_territory';

const member = (id: string, affinity: number | null): RegionMember => ({
	id,
	title: id,
	doc_type: 'concept',
	affinity
});

/** The interior box the TierTerritory renders into (viewBox W × H-60). */
const BOX = { width: 1040, height: 560 };
/** Must track MAX_R in regionInterior.ts — the legibility cap that stops circles
 *  from being scaled up to fill the canvas. */
const MAX_R = 58;

const insideBox = (
	out: ReturnType<typeof packRegionMembers>,
	box: { width: number; height: number }
) => {
	for (const p of out) {
		expect(p.x - p.r).toBeGreaterThanOrEqual(-0.01);
		expect(p.x + p.r).toBeLessThanOrEqual(box.width + 0.01);
		expect(p.y - p.r).toBeGreaterThanOrEqual(-0.01);
		expect(p.y + p.r).toBeLessThanOrEqual(box.height + 0.01);
	}
};

const overlaps = (a: { x: number; y: number; r: number }, b: { x: number; y: number; r: number }) =>
	Math.hypot(a.x - b.x, a.y - b.y) < a.r + b.r - 0.5;

describe('packRegionMembers', () => {
	it('returns one positioned circle per member, fully inside the box', () => {
		const out = packRegionMembers([member('a', 0.9), member('b', 0.3), member('c', null)], BOX);
		expect(out).toHaveLength(3);
		insideBox(out, BOX);
		for (const p of out) expect(p.r).toBeGreaterThan(0);
	});

	it('bounds circle size — a sparse region is NOT inflated to fill the canvas', () => {
		// The Beat-2a regression: 2–3 members packed with pack().size() became
		// canvas-filling, overlapping blobs. Bounded radii cap every chip at MAX_R.
		for (const count of [1, 2, 3]) {
			const out = packRegionMembers(
				Array.from({ length: count }, (_, i) => member(`m${i}`, null)),
				BOX
			);
			for (const p of out) expect(p.r).toBeLessThanOrEqual(MAX_R + 0.01);
		}
	});

	it('sizes radius monotonically with affinity when present', () => {
		const out = packRegionMembers([member('hi', 0.9), member('lo', 0.05)], { width: 600, height: 600 });
		const r = (id: string) => out.find((p) => p.id === id)!.r;
		expect(r('hi')).toBeGreaterThan(r('lo'));
	});

	it('gives null-affinity members a legible neutral size (not the floor)', () => {
		// Real data has affinity uniformly null; those chips must stay legible, sitting
		// between a low- and high-affinity chip rather than collapsing to the minimum.
		const out = packRegionMembers(
			[member('hi', 0.95), member('none', null), member('lo', 0.0)],
			{ width: 600, height: 600 }
		);
		const r = (id: string) => out.find((p) => p.id === id)!.r;
		expect(r('none')).toBeGreaterThan(r('lo'));
		expect(r('none')).toBeLessThanOrEqual(MAX_R + 0.01);
	});

	it('packs a crowded region without overlap and inside the box', () => {
		const out = packRegionMembers(
			Array.from({ length: 40 }, (_, i) => member(`m${i}`, null)),
			BOX
		);
		expect(out).toHaveLength(40);
		insideBox(out, BOX);
		for (let i = 0; i < out.length; i++) {
			for (let j = i + 1; j < out.length; j++) {
				expect(overlaps(out[i], out[j])).toBe(false);
			}
		}
	});

	it('returns [] for no members', () => {
		expect(packRegionMembers([], { width: 10, height: 10 })).toEqual([]);
	});
});
