import { describe, expect, test } from 'vitest';
import type { AtlasEntry } from '$lib/types/generated/graph_atlas';
import { declareEntryBounds, renderBoundLine } from './bound';
import { buildEntryGraph } from './model';

/**
 * Chunk A/C — the entry read, folded into marks.
 *
 * The defect being replaced drew 250 marks of which 244 had their edges dropped, because the drawn
 * set and the walked set were chosen by unrelated criteria. What makes that unrepresentable here is
 * that the server returns the INDUCED subgraph, so this function has no filtering left to do — and
 * the tests below are about what the fold must NOT do to it.
 */

const node = (id: string, degree: number, o: Partial<AtlasEntry['nodes'][number]> = {}) => ({
	id,
	title: id.toUpperCase(),
	doc_type: 'task',
	home: 'context' as const,
	degree,
	salience: 0.9,
	excerpt: null,
	stage: null,
	home_id: 'ctx-1',
	updated: '2026-08-20T10:00:00Z',
	...o,
});

const edge = (source: string, target: string, weight = 2) => ({
	id: `${source}-${target}`,
	source,
	target,
	edge_kind: 'contains' as const,
	polarity: 'forward' as const,
	label: 'parent_of',
	weight,
});

const homes = new Map([
	['ctx-1', '@me/temper'],
	['map-1', 'Temper — self-cognition'],
]);

describe('folding the entry read', () => {
	const entry: AtlasEntry = {
		nodes: [node('a', 12), node('b', 3), node('c', 7)],
		edges: [edge('a', 'b'), edge('a', 'c')],
		bounds: { drawn: 3, eligible: 40, in_scope: 100, truncated: true },
	};

	test('degree is recomputed over the DRAWN edges, never taken from the wire', () => {
		// `AtlasNode.degree` is the CORPUS degree — how connected this is in everything you can see.
		// The screen must show how connected it is in what you are LOOKING AT. Both are legitimate
		// and they are different quantities; a node reading `12` beside two strokes gives a reader
		// nothing to do but doubt themselves.
		const model = buildEntryGraph(entry, homes);
		const byId = new Map(model.nodes.map((n) => [n.id, n]));
		expect(byId.get('a')!.degree).toBe(2);
		expect(byId.get('b')!.degree).toBe(1);
		expect(byId.get('c')!.degree).toBe(1);
	});

	test('salience never reaches a node', () => {
		// `no-derived-thing-poses-as-authored` — the clause that got the tier model deleted.
		// Salience is region-DERIVED and rides along on every graph read, so it is dropped at the
		// fold rather than left for a mark to accidentally size on.
		const model = buildEntryGraph(entry, homes);
		for (const n of model.nodes) {
			expect(Object.keys(n)).not.toContain('salience');
		}
	});

	test('a node names where it lives, resolved from the anchors the page already read', () => {
		const model = buildEntryGraph(entry, homes);
		expect(model.nodes[0].homeRef).toBe('@me/temper');
	});

	test('an unresolvable home says so rather than inventing one', () => {
		const orphan = buildEntryGraph(
			{ ...entry, nodes: [node('x', 1, { home_id: 'unknown-anchor' })] },
			homes,
		);
		expect(orphan.nodes[0].homeRef).toBeNull();
	});

	test('edges carry their real weight — AtlasEdge stores one, ViaEntry does not', () => {
		const model = buildEntryGraph(entry, homes);
		expect(model.edges[0].weight).toBe(2);
		expect(model.edges[0].seedIds).toEqual([]);
	});

	test('no via entries collapsed, and zero is the honest count', () => {
		expect(buildEntryGraph(entry, homes).viaEntries).toBe(0);
	});

	test('the read declares ONE arm, and it describes the read rather than the reader', () => {
		// The entry read ranked the reader's whole visible corpus by connectedness and drew the top
		// of it. Nothing here was asked for. It used to mark all of it `'seed'` and inherit
		// `describeArm`'s sentence for a composition — *"In the places you asked about"* — on a
		// screen where the box was empty, on all 130 cards and on the single a11y heading.
		//
		// @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §1.3a
		const model = buildEntryGraph(entry, homes);

		expect(model.arms).toHaveLength(1);
		expect(model.arms[0].label).toBe('What your work is built around');
		for (const arm of model.arms) {
			expect(arm.label.toLowerCase()).not.toContain('asked');
			expect(arm.label.toLowerCase()).not.toContain('question');
		}
	});

	test('every mark carries a key this read declared — no node points outside its own legend', () => {
		const model = buildEntryGraph(entry, homes);
		const declared = new Set(model.arms.map((a) => a.key));

		for (const n of model.nodes) expect(declared.has(n.arm)).toBe(true);
	});

	test('its one arm is where the read STOOD, so nothing is drawn as reached', () => {
		// One arm means no contrast, so `armsDistinguish` withdraws the ring — unchanged from #741.
		// `reached: false` is what keeps every mark in the force core, which used to be the global
		// `arm !== 'walk'` check that no per-view vocabulary could have satisfied.
		expect(buildEntryGraph(entry, homes).arms.every((a) => !a.reached)).toBe(true);
	});

	test('there is no ResourceView behind an entry mark, and none is fabricated', () => {
		// A synthesised row would put invented values on a hover card, which is the exact defect
		// class this surface is being repaired for.
		for (const n of buildEntryGraph(entry, homes).nodes) {
			expect(n.resource).toBeNull();
		}
	});
});

describe('the bound line declares what was not drawn', () => {
	const places = { asked: 1, available: 12 };

	test('the unconnected remainder is stated, never folded into one number', () => {
		// `legibility-is-never-bought-with-silent-omission` is the clause this goal sits under. On
		// the corpus that produced this design the remainder is 1,077 of 3,574 — quietly dropping
		// them would be a bigger silence than the 244-of-250 band it replaced.
		const line = renderBoundLine(
			declareEntryBounds({ drawn: 130, eligible: 2497, inScope: 3574, truncated: true }, places),
		);
		expect(line).toContain('130 of 2497 connected');
		expect(line).toContain('1077 unconnected not drawn');
	});

	test('a fully connected corpus makes no remainder claim', () => {
		const line = renderBoundLine(
			declareEntryBounds({ drawn: 20, eligible: 20, inScope: 20, truncated: false }, places),
		);
		expect(line).toContain('20 of 20 connected');
		expect(line).not.toContain('unconnected');
	});

	test('the axes a composition would have filled are ABSENT, not zero', () => {
		// This view ran no funnel, so groupings is the third state — not a width of none — and it
		// followed nothing on, so that axis is absent rather than `0 rows`. Reporting zeros would
		// describe a composition that returned nothing, a different claim about the reader's corpus
		// than one that was never run.
		const d = declareEntryBounds({ drawn: 5, eligible: 5, inScope: 9, truncated: false }, places);
		expect(d.followedOn).toBeNull();
		expect(d.fromYourPlaces).toBeNull();
		expect(d.inYourPlaces).toBeNull();
		expect(d.groupings).toEqual({ applicable: false });
		expect(renderBoundLine(d)).toContain('groupings not applicable');
	});

	test('the line is present even when nothing is partial — complete is TOLD, not inferred', () => {
		const line = renderBoundLine(
			declareEntryBounds({ drawn: 3, eligible: 3, inScope: 3, truncated: false }, places),
		);
		expect(line.startsWith('Showing ')).toBe(true);
		expect(line).toContain('1 of 12 places');
	});
});

describe('the corpus figure is kept beside the derived one, never merged into it', () => {
	/**
	 * `AtlasNode.degree` was already on the wire and already the CORPUS degree — the fold simply
	 * dropped it. §5.3 originally ruled it must never reach the screen; that ruling was
	 * `[narrowed — 2026-08-21, Pete]` to *"only inside a sentence that states its relationship to
	 * the drawn one"*, because the entry read's band is otherwise undescribable: every member of
	 * it carries corpus degree ≥ the cut, so `0 links` is false for all of them.
	 *
	 * The two quantities keep two names. That is the whole of what §5.3 was protecting.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §4, §5.1
	 */
	const strandedHub: AtlasEntry = {
		// `a`—`b` is the only edge, so `c` is drawn with 87 corpus connections and no stroke: the
		// production shape of `Maintenance` at K=130, in miniature.
		nodes: [node('a', 12), node('b', 3), node('c', 87)],
		edges: [edge('a', 'b')],
		bounds: { drawn: 3, eligible: 2499, in_scope: 3583, truncated: true },
	};

	test('the wire figure is carried under its own name', () => {
		const byId = new Map(buildEntryGraph(strandedHub, homes).nodes.map((n) => [n.id, n]));
		expect(byId.get('c')!.corpusDegree).toBe(87);
		expect(byId.get('a')!.corpusDegree).toBe(12);
	});

	test('and the derived one is still what it was — the two disagree on purpose', () => {
		const byId = new Map(buildEntryGraph(strandedHub, homes).nodes.map((n) => [n.id, n]));
		// The hub the whole repair exists for: nothing drawn, 87 in the corpus.
		expect(byId.get('c')!.degree).toBe(0);
		expect(byId.get('c')!.corpusDegree).toBe(87);
		// And a connected node's derived count is NOT its corpus count either.
		expect(byId.get('a')!.degree).toBe(1);
		expect(byId.get('a')!.corpusDegree).toBe(12);
	});
});
