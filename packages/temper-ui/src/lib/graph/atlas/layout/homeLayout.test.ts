import { describe, expect, it } from 'vitest';
import { layoutHome } from './homeLayout';
import type { HomeCogmap, HomeTeam } from '$lib/types/generated/graph_home';

const team = (id: string, name: string = id): HomeTeam =>
	({ id, slug: name, name, resource_count: 0, cogmap_count: 0 });
const cogmap = (id: string, team_ids: string[]): HomeCogmap =>
	({ id, name: id, team_ids, region_count: 0, facet_count: 0 });

describe('layoutHome', () => {
	it('places you at left, teams in a right column, one edge each', () => {
		const g = layoutHome([team('t1', 'A'), team('t2', 'B'), team('t3', 'C')], [], { width: 800, height: 400 });
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
		const g = layoutHome([], [], { width: 400, height: 300 });
		expect(g.teams).toEqual([]);
		expect(g.edges).toEqual([]);
	});
});

describe('layoutHome — cogmap column', () => {
	it('places cogmaps in a third column right of teams', () => {
		const g = layoutHome([team('t1')], [cogmap('c1', ['t1'])], { width: 1000, height: 600 });
		const t = g.teams.find((n) => n.kind === 'team')!;
		const c = g.cogmaps.find((n) => n.kind === 'cogmap')!;
		expect(c.x).toBeGreaterThan(t.x);
	});

	it('draws one team→cogmap edge per membership (shared cogmap = 2 edges)', () => {
		const g = layoutHome(
			[team('t1'), team('t2')],
			[cogmap('shared', ['t1', 't2'])],
			{ width: 1000, height: 600 }
		);
		expect(g.cogmapEdges).toHaveLength(2);
	});

	it('renders a cogmap with no visible team edge (no left edge)', () => {
		const g = layoutHome([team('t1')], [cogmap('lonely', [])], { width: 1000, height: 600 });
		expect(g.cogmaps).toHaveLength(1);
		expect(g.cogmapEdges).toHaveLength(0);
	});
});
