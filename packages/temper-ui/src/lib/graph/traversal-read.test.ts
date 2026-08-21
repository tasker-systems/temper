import { describe, expect, test } from 'vitest';
import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
import { declareTraversalBounds, renderBoundLine } from './bound';
import { buildTraversal, ENTRY_ARMS, TRAVERSAL_ARMS } from './model';
import { armsDistinguish } from './presentation';

/**
 * Chunk D2 — the traversal read, folded into marks, and what it owes the reader.
 *
 * **Read this before adding an assertion here.** This read declares its own arms AND its own bound
 * line, so it is trivially able to validate itself: a test that derives what it expects from
 * `model.arms` or from the `BoundDeclaration` agrees with the code whatever either says. That is
 * not hypothetical — the D1 session shipped exactly that test, probed it by flipping the
 * declaration, and watched 618 assertions pass while the defect was back on screen.
 *
 * So every assertion below is grounded in one of three things the traversal did **not** declare:
 *
 * 1. **`seeds`** — an argument, chosen by the caller from the URL.
 * 2. **the fixture's own node ids** — an input.
 * 3. **a hand-written string**, including what must NOT appear in one.
 *
 * Where a test does read the declaration, it is pinning the declaration *itself* and says so.
 */

const node = (id: string, degree: number, o: Record<string, unknown> = {}) => ({
	id,
	title: id.toUpperCase(),
	doc_type: 'task',
	home: 'context' as const,
	degree,
	salience: 0.9,
	excerpt: null,
	stage: null,
	home_id: 'ctx-1',
	updated: '2026-08-21T10:00:00Z',
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

const homes = new Map([['ctx-1', '@me/temper']]);

/**
 * One hop from `a`, which reached `b` and `c`.
 *
 * The seed is FIRST in `nodes`, which is what the service does — *"Seeds FIRST, then the walked
 * endpoints. A seed that reached nothing still renders"* (`graph_service.rs`). Nothing in the
 * payload marks it as the seed, and that is the point: `AtlasSubgraph` is `{ nodes, edges }`.
 */
const walked: AtlasSubgraph = {
	nodes: [node('a', 22), node('b', 3), node('c', 7)],
	edges: [edge('a', 'b'), edge('a', 'c')],
};

describe('folding a traversal', () => {
	test('which arm a mark lands in is decided by the SEEDS, which the response does not carry', () => {
		// The grounding for this assertion is the array passed in, not anything the model says. If
		// `buildTraversal` put every node in one arm — the shape that would silence the ring — the
		// two sides of this comparison would be equal.
		const model = buildTraversal(walked, ['a'], homes);
		const armOf = (id: string) => model.nodes.find((n) => n.id === id)?.arm;

		expect(armOf('a')).not.toBe(armOf('b'));
		expect(armOf('b')).toBe(armOf('c'));
	});

	test('hopping from a DIFFERENT mark moves the ring, on an identical response', () => {
		// The strongest available statement that the seeds decide this and the payload does not:
		// same `walked` both times, and the mark that stands apart follows the argument.
		const fromA = buildTraversal(walked, ['a'], homes);
		const fromB = buildTraversal(walked, ['b'], homes);

		const standing = (m: ReturnType<typeof buildTraversal>) =>
			m.nodes.filter((n) => !m.arms.find((x) => x.key === n.arm)?.reached).map((n) => n.id);

		expect(standing(fromA)).toEqual(['a']);
		expect(standing(fromB)).toEqual(['b']);
	});

	test('the ring lights, because the two arms actually have members', () => {
		// `armsDistinguish` counts the MARKS, never the legend — a read may declare an arm and
		// return nothing for it. Grounded in the node list, which is the fixture's.
		expect(armsDistinguish(buildTraversal(walked, ['a'], homes).nodes)).toBe(true);
	});

	test('a seed the reader cannot see withdraws the ring rather than ringing nothing', () => {
		// `hydrate_atlas_nodes_visible` drops a seed this reader may not read, so the response
		// simply does not contain it. Every mark is then something the walk reached, there is no
		// contrast, and a ring drawn anyway would encode a constant — #741's defect, restaged.
		const model = buildTraversal(walked, ['not-visible-to-me'], homes);

		expect(model.nodes.every((n) => n.arm === TRAVERSAL_ARMS[1].key)).toBe(true);
		expect(armsDistinguish(model.nodes)).toBe(false);
	});

	// The core/periphery half of this — `coreOf` in `GraphCanvas.svelte` — is not reachable from
	// here: it is a closure over the model inside the component, not an exported function. It is
	// witnessed on the rendered DOM in `GraphPage.component.test.ts` instead, which is also where
	// the ring itself is counted.

	test('degree is recomputed over the DRAWN edges, never taken from the wire', () => {
		// Same rule the entry read follows. `a` arrives with corpus degree 22 and is drawn with two
		// strokes; a mark reading 22 beside two lines gives a reader nothing to do but doubt
		// themselves. Both quantities survive, under two names.
		const model = buildTraversal(walked, ['a'], homes);
		const byId = new Map(model.nodes.map((n) => [n.id, n]));

		expect(byId.get('a')!.degree).toBe(2);
		expect(byId.get('a')!.corpusDegree).toBe(22);
		expect(byId.get('b')!.degree).toBe(1);
	});

	test('no edge claims a seed reached it, because the read does not report which one did', () => {
		// `graph_induced_edges` returns an induced edge set, not per-edge provenance. Filling
		// `seedIds` with every seed would let the rail say "reached from X" about a path nothing
		// traced.
		expect(buildTraversal(walked, ['a'], homes).edges.every((e) => e.seedIds.length === 0)).toBe(
			true,
		);
	});
});

describe('the arm vocabulary itself — pinned, because nothing else in this file reads it', () => {
	test('exactly one arm is where the reader stands, and it is the one seeds land in', () => {
		// This test DOES read the declaration; that is its whole job. Every other test above would
		// still pass if both arms declared `reached: true`, and the ring would then vanish.
		expect(TRAVERSAL_ARMS.filter((a) => !a.reached)).toHaveLength(1);
		expect(TRAVERSAL_ARMS.find((a) => !a.reached)?.key).toBe(TRAVERSAL_ARMS[0].key);
	});

	test('no read reaches another read’s words', () => {
		// The class the whole of D was pointed at: `describeArm`'s global switch is gone, so the
		// entry read cannot be drawn under a traversal's label or the reverse. Compared across two
		// declarations, which is the only place this is observable.
		const entry = ENTRY_ARMS.map((a) => a.label);
		const traversal = TRAVERSAL_ARMS.map((a) => a.label);

		expect(traversal.some((l) => entry.includes(l))).toBe(false);
		// The string a reader filed, and the one D1 removed. It belongs to a composition and to
		// nothing else; a traversal asserting it would be the same defect a third time.
		expect(traversal.join(' ')).not.toMatch(/asked about/i);
	});
});

describe('what a traversal declares about its own bounds', () => {
	const line = (o: { drawn: number; from: number; depth: number }) =>
		renderBoundLine(declareTraversalBounds(o));

	test('states no ratio, because it withheld nothing and has no denominator to state', () => {
		// The one assertion §5 asks for by name: "there is no `drawn of eligible` ratio to state and
		// one must not be manufactured." Written against the STRING, so it fails whatever axis a
		// future edit reaches for to build one.
		expect(line({ drawn: 51, from: 1, depth: 1 })).not.toMatch(/\bof\b/);
	});

	test('claims no place scope, because the walk had none', () => {
		// §10.3: the walk runs over the reader's whole visible corpus. `12 of 12 places` would not
		// be a smaller number, it would be a false claim of confinement.
		const rendered = line({ drawn: 51, from: 1, depth: 1 });

		expect(rendered).not.toMatch(/place/i);
		expect(rendered).not.toMatch(/grouping/i);
	});

	test('is still on screen when nothing was withheld — chrome, not a warning', () => {
		// §7.1: "present whether or not the view is partial, so complete is something the reader is
		// TOLD rather than something they infer from silence."
		expect(line({ drawn: 51, from: 1, depth: 1 })).toBe(
			'Showing 51 marks · 1 you hopped from · complete within 1 hop · deeper not reported',
		);
	});

	test('says it is complete at this depth AND that it cannot see past it', () => {
		// Two different states of knowledge, and neither may be collapsed into the other: the read
		// genuinely returned everything at this depth, and genuinely cannot say what a further hop
		// finds.
		const rendered = line({ drawn: 51, from: 1, depth: 2 });

		expect(rendered).toContain('complete within 2 hops');
		expect(rendered).toContain('deeper not reported');
	});

	test('reports a seed it could not draw rather than going quiet about it', () => {
		// Zero here is a real state, not a missing measurement — and it is the reason nothing on
		// screen carries a ring. Silence would leave the reader to work that out.
		expect(line({ drawn: 4, from: 0, depth: 1 })).toContain(
			'nothing you can see to have hopped from',
		);
	});

	test('the counts describe THIS screen, with nothing borrowed from a composition', () => {
		// §7.1's opening: the line "must not keep displaying the grounding query's counts — on hop
		// three those describe a screen the reader is no longer looking at." Every composition axis
		// is absent, and absent is checked here rather than assumed from the render.
		const d = declareTraversalBounds({ drawn: 51, from: 1, depth: 1 });

		expect(d.places).toBeNull();
		expect(d.groupings).toBeNull();
		expect(d.inYourPlaces).toBeNull();
		expect(d.fromYourPlaces).toBeNull();
		expect(d.followedOn).toBeNull();
		expect(d.orientation).toBeNull();
	});

	test('singular and plural both read as English', () => {
		expect(line({ drawn: 1, from: 1, depth: 1 })).toContain('1 mark ·');
		expect(line({ drawn: 2, from: 1, depth: 1 })).toContain('2 marks ·');
	});
});
