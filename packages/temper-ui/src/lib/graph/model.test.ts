import { describe, expect, test } from 'vitest';
import type { QueryResponse, ResourceHit, ViaEntry } from '$lib/types/generated/query';
import type { ResourceView } from '$lib/types/generated/resource_view';
import type { GraphPlan } from './composition';
import { buildGraph, excerptOf } from './model';

/**
 * A row as every read answers — `list`, `show`, both search arms. A context-homed row has NO
 * `cogmap_id` KEY at all (`skip_serializing_if`), which is why the fixture omits it rather than
 * nulling it: a null would let a `!==` comparison pass that the real wire would fail.
 */
const row = (o: {
	id: string;
	title?: string;
	docType?: string;
	cogmap?: boolean;
	body?: string;
}) =>
	({
		id: o.id,
		ref: `${o.id}-ref`,
		title: o.title ?? o.id,
		doc_type_name: o.docType ?? 'task',
		...(o.cogmap ? { cogmap_id: 'map-1', cogmap_name: 'A map' } : { context_slug: 'temper' }),
		managed_meta: {},
		content: o.body ?? null,
	}) as unknown as ResourceView;

const via = (o: {
	seed: string;
	source: string;
	target: string;
	kind?: string;
	label?: string | null;
}): ViaEntry =>
	({
		seed_id: o.seed,
		source_id: o.source,
		target_id: o.target,
		edge_kind: o.kind ?? 'contains',
		label: 'label' in o ? o.label : 'parent_of',
		polarity: 'forward',
	}) as unknown as ViaEntry;

const hit = (resource: ResourceView, vias: ViaEntry[] = []): ResourceHit =>
	({
		resource,
		scoring: { score_kind: 'graph_score', score: 1 },
		located_at: null,
		via: vias,
	}) as unknown as ResourceHit;

const plan = (o: { surveys?: string[]; walk?: string }): GraphPlan =>
	({
		composition: { outcome: { returns: [] }, stages: [] },
		anchorsAsked: [],
		anchorsAvailable: 0,
		surveyStages: o.surveys ?? [],
		walkStage: o.walk ?? 'w',
	}) as GraphPlan;

const response = (returned: Record<string, unknown>): QueryResponse =>
	({ returned, trace: { stages: [] } }) as unknown as QueryResponse;

const resources = (hits: ResourceHit[]) => ({
	act: 'follow-from',
	produced: { produced: 'resources', hits },
	extent: { extent: 'complete' },
	terms_applied: {},
	disclosed_regions: [],
});

describe('the mark vocabulary is exactly node and edge', () => {
	test('the model carries no third collection — a third mark cannot be added silently', () => {
		const model = buildGraph({ response: response({}), plan: plan({}), seeds: [] });

		// `arms` is a LEGEND, not marks: it names how to talk about the nodes, and nothing on the
		// canvas is drawn from it. `nodes` and `edges` are still the only two things drawn, which
		// is the property this pins — any new key here is a reviewable act, which is the point.
		expect(Object.keys(model).sort()).toEqual(['arms', 'edges', 'nodes', 'viaEntries']);
	});

	test('a stage that produced REGIONS contributes no node', () => {
		// The structural half of `no-derived-thing-poses-as-authored`: the discriminant is
		// checked, so a region hit cannot reach the canvas even if a stage this surface draws
		// starts producing them.
		const model = buildGraph({
			response: response({
				w: {
					act: 'survey',
					produced: { produced: 'regions', hits: [{ region: { region_id: 'r' }, scoring: {} }] },
					extent: { extent: 'complete' },
				},
			}),
			plan: plan({}),
			seeds: [],
		});

		expect(model.nodes).toEqual([]);
	});
});

describe('via collapses to distinct edges before anything is drawn', () => {
	// The measured shape, from the real 50-node walk: 1,973 (seed, edge) entries over 102
	// distinct edges — 19.3x. Undeduped, that is 1,973 marks where 102 belong.
	const a = row({ id: 'a' });
	const b = row({ id: 'b' });

	test('one edge reached from many seeds is drawn once', () => {
		const seeds = ['s1', 's2', 's3', 's4', 's5'];
		const model = buildGraph({
			response: response({
				w: resources([
					hit(a),
					hit(
						b,
						seeds.map((s) => via({ seed: s, source: 'a', target: 'b' })),
					),
				]),
			}),
			plan: plan({}),
			seeds: [],
		});

		expect(model.viaEntries).toBe(5);
		expect(model.edges).toHaveLength(1);
		expect(model.edges[0].seedIds).toEqual(seeds);
	});

	test('the four fields §3 names are the identity — a different label is a different edge', () => {
		const vias = [
			via({ seed: 's', source: 'a', target: 'b', label: 'parent_of' }),
			via({ seed: 's', source: 'a', target: 'b', label: 'leads_to' }),
			via({ seed: 's', source: 'a', target: 'b', kind: 'near', label: 'parent_of' }),
			via({ seed: 's', source: 'b', target: 'a', label: 'parent_of' }),
		];
		const model = buildGraph({
			response: response({ w: resources([hit(a), hit(b, vias)]) }),
			plan: plan({}),
			seeds: [],
		});

		expect(model.viaEntries).toBe(4);
		expect(model.edges).toHaveLength(4);
	});

	test('a null label is its own identity and does not collide with a labelled edge', () => {
		const model = buildGraph({
			response: response({
				w: resources([
					hit(a),
					hit(b, [
						via({ seed: 's', source: 'a', target: 'b', label: null }),
						via({ seed: 's', source: 'a', target: 'b', label: 'parent_of' }),
					]),
				]),
			}),
			plan: plan({}),
			seeds: [],
		});

		expect(model.edges).toHaveLength(2);
	});

	test('an edge whose other end is not drawn is not a stroke to nowhere', () => {
		const model = buildGraph({
			response: response({
				w: resources([hit(a, [via({ seed: 's', source: 'a', target: 'off-screen' })])]),
			}),
			plan: plan({}),
			seeds: [],
		});

		expect(model.viaEntries).toBe(1);
		expect(model.edges).toEqual([]);
	});

	test('degree counts DEDUPED edges, so the density target is 25 and not 98', () => {
		// The same measurement's other half: the highest-degree node carried 98 via entries
		// over 25 distinct edges. Sizing a mark on the raw count would inflate it ~4x.
		const hub = row({ id: 'hub' });
		const peers = Array.from({ length: 25 }, (_, i) => row({ id: `p${i}` }));
		const vias = peers.flatMap((p) =>
			['s1', 's2', 's3', 's4'].map((s) => via({ seed: s, source: 'hub', target: p.id })),
		);

		const model = buildGraph({
			response: response({ w: resources([hit(hub, vias), ...peers.map((p) => hit(p))]) }),
			plan: plan({}),
			seeds: [],
		});

		expect(model.viaEntries).toBe(100);
		expect(model.edges).toHaveLength(25);
		expect(model.nodes.find((n) => n.id === 'hub')?.degree).toBe(25);
		expect(model.nodes.find((n) => n.id === 'p0')?.degree).toBe(1);
	});
});

describe('a node keeps the arm nearest the reader', () => {
	const r = row({ id: 'x' });

	test('a seed the walk also reached is still the reader’s own seed', () => {
		const model = buildGraph({
			response: response({ w: resources([hit(r)]) }),
			plan: plan({}),
			seeds: [r],
		});

		expect(model.nodes).toHaveLength(1);
		expect(model.nodes[0].arm).toBe('seed');
	});

	test('a survey row the walk also returned stays on the survey arm', () => {
		const model = buildGraph({
			response: response({ s1: resources([hit(r)]), w: resources([hit(r)]) }),
			plan: plan({ surveys: ['s1'] }),
			seeds: [],
		});

		expect(model.nodes).toHaveLength(1);
		expect(model.nodes[0].arm).toBe('survey');
	});

	test('a row only the walk reached is on the walk arm', () => {
		const model = buildGraph({
			response: response({ w: resources([hit(r)]) }),
			plan: plan({}),
			seeds: [],
		});

		expect(model.nodes[0].arm).toBe('walk');
	});
});

describe('a node is the resource, projected as the marks already read it', () => {
	test('home comes from which anchor the row is homed by, not from its doc-type', () => {
		const model = buildGraph({
			response: response({}),
			plan: plan({}),
			seeds: [
				row({ id: 'c', docType: 'session' }),
				row({ id: 'm', docType: 'session', cogmap: true }),
			],
		});

		expect(model.nodes.map((n) => n.home)).toEqual(['context', 'cogmap']);
	});

	test('the whole row travels with the node, so a panel needs no second read', () => {
		const model = buildGraph({
			response: response({}),
			plan: plan({}),
			seeds: [row({ id: 'a', title: 'A title' })],
		});

		expect(model.nodes[0].resource?.ref).toBe('a-ref');
		expect(model.nodes[0].title).toBe('A title');
	});
});

describe('an excerpt is derived only from a body that was actually requested', () => {
	test('a row with no body has no excerpt — absent is not empty', () => {
		expect(excerptOf(row({ id: 'a' }))).toBeNull();
	});

	test('an empty body is still no excerpt', () => {
		expect(excerptOf(row({ id: 'a', body: '' }))).toBeNull();
	});

	test('the first paragraph only, whitespace collapsed', () => {
		expect(excerptOf(row({ id: 'a', body: 'One   line\nand more.\n\nSecond para.' }))).toBe(
			'One line and more.',
		);
	});

	test('a long paragraph is truncated at a word boundary', () => {
		const words = `${'alpha '.repeat(80)}`;
		const out = excerptOf(row({ id: 'a', body: words }), 40);

		expect(out?.endsWith('…')).toBe(true);
		expect(out?.length).toBeLessThanOrEqual(41);
		expect(out).not.toContain('alph…');
	});
});

describe('the composition path reports no corpus degree, and says so', () => {
	/**
	 * `ResourceView` carries no degree — verified: nothing named `degree` exists on the generated
	 * type. So this path genuinely cannot report how connected a resource is in the corpus, and
	 * `null` says *not reported* rather than *zero*.
	 *
	 * This is what stops the entry read's band sentence — *"but each connects to N things
	 * elsewhere in your corpus"* — from appearing on a screen whose read never measured it.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.2
	 */
	test('a node from a composition answer reports its corpus degree as absent', () => {
		const model = buildGraph({
			response: response({}),
			plan: plan({}),
			seeds: [row({ id: 'a' })],
		});

		expect(model.nodes[0].corpusDegree).toBeNull();
	});

	test('every node on that path, not just the first', () => {
		const model = buildGraph({
			response: response({}),
			plan: plan({}),
			seeds: [row({ id: 'a' }), row({ id: 'b' }), row({ id: 'c' })],
		});

		expect(model.nodes.every((n) => n.corpusDegree === null)).toBe(true);
	});
});
