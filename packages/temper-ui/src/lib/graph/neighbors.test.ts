// neighbors.test.ts
import { describe, expect, it } from 'vitest';
import type { AtlasEdge, AtlasNode } from '$lib/types/generated/graph_atlas';
import { atlasNeighbors } from './neighbors';

const node = (o: Partial<AtlasNode>): AtlasNode => ({
	id: 'x',
	title: 'X',
	doc_type: null,
	home: 'context',
	degree: 0,
	salience: null,
	excerpt: null,
	stage: null,
	home_id: null,
	updated: null,
	...o,
});
const edge = (o: Partial<AtlasEdge>): AtlasEdge => ({
	id: 'e',
	source: 's',
	target: 't',
	edge_kind: 'contains',
	polarity: 'forward',
	label: null,
	weight: 1,
	...o,
});

describe('atlasNeighbors', () => {
	it('yields out/in neighbors, coalescing label ?? edge_kind', () => {
		const nodes = [node({ id: 'a', title: 'A' }), node({ id: 'b', title: 'B' })];
		const edges = [
			edge({ id: 'e1', source: 'a', target: 'b', label: null, edge_kind: 'contains' }),
		];
		const r = atlasNeighbors('a', nodes, edges);
		expect(r).toEqual([{ dir: '→', label: 'contains', other: nodes[1], key: expect.any(String) }]);
	});
	it('drops edges whose other end is absent', () => {
		expect(
			atlasNeighbors('a', [node({ id: 'a' })], [edge({ source: 'a', target: 'ghost' })]),
		).toEqual([]);
	});
	/**
	 * `[found on production — 2026-08-21]` The rail keyed its render on `other.id + label + dir`.
	 * **43 of the entry read's 275 edges share a pair** — a resource can `relates_to` another AND
	 * `derived_from` it — and where two of them coalesce to the same displayed label the keys were
	 * identical, Svelte threw `each_key_duplicate`, and the whole page went blank. 5 of 130 nodes.
	 *
	 * The shape below is taken from that measurement, not invented: two edges, same pair, same
	 * direction, same effective label, DIFFERENT kind. All 275 production edges are distinct under
	 * the four-field identity, which is why the repair is to key on it rather than to dedupe.
	 */
	it('tells two edges between the same pair apart, even when they read the same', () => {
		const nodes = [node({ id: 'a' }), node({ id: 'b', title: 'B' })];
		const edges = [
			edge({ id: 'e1', source: 'a', target: 'b', label: 'supports', edge_kind: 'leads_to' }),
			edge({ id: 'e2', source: 'a', target: 'b', label: 'supports', edge_kind: 'contains' }),
		];

		const r = atlasNeighbors('a', nodes, edges);

		expect(r).toHaveLength(2);
		// Both rows say the same thing to a reader — and are two different edges.
		expect(r.map((n) => n.label)).toEqual(['supports', 'supports']);
		expect(new Set(r.map((n) => n.key)).size).toBe(2);
	});

	it('a null label still keys apart from a real label that matches its kind', () => {
		// The other half of `label ?? edge_kind`: an absent label displays as the kind, so it can
		// collide with an edge whose label IS that kind. Distinct rows in the substrate.
		const nodes = [node({ id: 'a' }), node({ id: 'b' })];
		const edges = [
			edge({ id: 'e1', source: 'a', target: 'b', label: null, edge_kind: 'contains' }),
			edge({ id: 'e2', source: 'a', target: 'b', label: 'contains', edge_kind: 'leads_to' }),
		];

		const r = atlasNeighbors('a', nodes, edges);
		expect(new Set(r.map((n) => n.key)).size).toBe(2);
	});

	it('sorts by label then title deterministically', () => {
		const nodes = [
			node({ id: 'a' }),
			node({ id: 'b', title: 'Beta' }),
			node({ id: 'c', title: 'Alpha' }),
		];
		const edges = [
			edge({ id: 'e1', source: 'a', target: 'b', label: 'rel' }),
			edge({ id: 'e2', source: 'a', target: 'c', label: 'rel' }),
		];
		expect(atlasNeighbors('a', nodes, edges).map((n) => n.other.title)).toEqual(['Alpha', 'Beta']);
	});
});
