import { describe, expect, it } from 'vitest';
import { layoutHome } from './homeLayout';
import type { TeamRow } from '$lib/types/generated/team';

const team = (id: string, name: string): TeamRow =>
	({ id, slug: name, name, description: null }) as unknown as TeamRow;

describe('layoutHome', () => {
	it('places you at left, teams in a right column, one edge each', () => {
		const g = layoutHome([team('t1', 'A'), team('t2', 'B'), team('t3', 'C')], { width: 800, height: 400 });
		expect(g.you.kind).toBe('you');
		expect(g.teams).toHaveLength(3);
		expect(g.edges).toHaveLength(3);
		for (const t of g.teams) {
			expect(t.x).toBeGreaterThan(g.you.x); // teams are to the right of you
			expect(t.x).toBeLessThanOrEqual(800);
			expect(t.y).toBeGreaterThanOrEqual(0);
			expect(t.y).toBeLessThanOrEqual(400);
		}
	});
	it('handles zero teams (you alone, no edges)', () => {
		const g = layoutHome([], { width: 400, height: 300 });
		expect(g.teams).toEqual([]);
		expect(g.edges).toEqual([]);
	});
});
