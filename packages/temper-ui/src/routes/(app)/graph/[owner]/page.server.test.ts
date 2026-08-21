// page.server.test.ts — the graph load, which had no test at all until this file.
//
// That absence is not incidental to what is tested here. `[observed on production — 2026-08-21]`
// clicking any mark on `/graph/@me` wrote `?sel=<uuid>` into the URL and opened nothing — for every
// node, on the screen every reader meets first, for as long as the entry read has existed. The
// cause was three hard-coded `null`s in one branch of this function, and **no test could have
// caught it**: the rail's tests all render `GraphPage` from a composition fixture, so they pass
// whether or not this load ever resolves a selection.
//
// **`vi.mock` over `$lib/server/*` is an EXTEND, not the house idiom** — the only other mocks in
// this codebase stand in for `$app/*`, and the one existing load test (`../../vault/[owner]/
// [context]/graph/shim.test.ts`) needs no I/O because the loader it exercises only reads `params`.
// It is spent deliberately: the defect lives in the load, and a witness anywhere else passes today.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AtlasEntry } from '$lib/types/generated/graph_atlas';

const readAnchorSources = vi.fn();
const readEntry = vi.fn();
const readResourceBody = vi.fn();
const readTrail = vi.fn();
const readTraversal = vi.fn();
const readSeedResources = vi.fn();
/**
 * Rejects by default, and that is the guard rather than an oversight: any test that takes the
 * composition branch by accident fails loudly instead of quietly answering. The routing tests below
 * assert on `mock.calls` rather than on a return value, so nothing needs it to resolve.
 */
const runComposition = vi.fn(() =>
	Promise.reject(new Error('this read must not run a composition')),
);

vi.mock('$lib/server/graph-query', () => ({
	readAnchorSources: (...a: unknown[]) => readAnchorSources(...a),
	readEntry: (...a: unknown[]) => readEntry(...a),
	readResourceBody: (...a: unknown[]) => readResourceBody(...a),
	readTraversal: (...a: unknown[]) => readTraversal(...a),
	readSeedResources: (...a: unknown[]) => readSeedResources(...a),
	runComposition: (...a: unknown[]) => runComposition(...(a as [])),
	readAnchorRegions: () => Promise.reject(new Error('these reads disclose no regions')),
}));
vi.mock('$lib/server/graph-reads', () => ({
	readTrail: (...a: unknown[]) => readTrail(...a),
}));

const { load } = await import('./+page.server');

const NODE = '019fa375-3844-71e1-a3a6-323320217e3d';

/** A resource the reader can see, in one context — enough for the plan to be buildable. */
const CONTEXTS = [
	{ id: 'ctx-1', owner_ref: '@me', slug: 'temper', resource_count: 42 },
] as unknown as Awaited<ReturnType<typeof readAnchorSources>>[0];

/**
 * Two marks, one edge. Shaped as the entry read really returns them — `AtlasNode`, so **no
 * `ResourceView` behind any of it**, which is the condition the rail has to work under.
 */
const ENTRY: AtlasEntry = {
	nodes: [
		{
			id: NODE,
			title: 'A goal',
			doc_type: 'goal',
			home: 'context',
			degree: 12,
			salience: 0.9,
			excerpt: null,
			stage: null,
			home_id: 'ctx-1',
			updated: '2026-08-20T10:00:00Z',
		},
		{
			id: 'other-node',
			title: 'Another',
			doc_type: 'task',
			home: 'context',
			degree: 3,
			salience: 0.4,
			excerpt: null,
			stage: null,
			home_id: 'ctx-1',
			updated: '2026-08-20T10:00:00Z',
		},
	],
	edges: [
		{
			id: 'e1',
			source: NODE,
			target: 'other-node',
			edge_kind: 'contains',
			polarity: 'forward',
			label: null,
			weight: 2,
		},
	],
	bounds: { drawn: 2, eligible: 40, in_scope: 100, truncated: true },
} as unknown as AtlasEntry;

/**
 * What `/api/graph/traverse` returns — an `AtlasSubgraph`, which is `{ nodes, edges }` and marks
 * **nothing** as a seed. Which mark the reader hopped from is knowable only from the address.
 */
const WALKED = {
	nodes: ENTRY.nodes,
	edges: ENTRY.edges,
} as unknown as { nodes: AtlasEntry['nodes']; edges: AtlasEntry['edges'] };

const run = (search = '') =>
	(load as (e: unknown) => Promise<Record<string, unknown>>)({
		locals: { accessToken: 'tok' },
		params: { owner: '@me' },
		url: new URL(`https://temperkb.io/graph/@me${search}`),
	});

beforeEach(() => {
	vi.clearAllMocks();
	readAnchorSources.mockResolvedValue([CONTEXTS, []]);
	readEntry.mockResolvedValue(ENTRY);
	readTraversal.mockResolvedValue(WALKED);
	readSeedResources.mockResolvedValue([]);
	runComposition.mockRejectedValue(new Error('this read must not run a composition'));
	readResourceBody.mockResolvedValue(null);
	readTrail.mockResolvedValue({ events: [] });
});

describe('the entry read resolves a selection, like every other read', () => {
	it('opens the rail on a mark the reader clicked — it opened nothing, for every node', async () => {
		const data = await run(`?sel=${NODE}`);

		expect(data.selected).toBe(NODE);
		expect(readResourceBody).toHaveBeenCalledWith('tok', NODE);
		expect(readTrail).toHaveBeenCalledWith('tok', 'node', NODE);
	});

	it('is still the ENTRY read — no composition runs behind the rail', async () => {
		// The mocked `runComposition` rejects, so taking that branch fails rather than quietly
		// answering. Guards the test above from passing on the wrong path.
		const data = await run(`?sel=${NODE}`);

		expect(readEntry).toHaveBeenCalledOnce();
		expect(data.readout).toBeNull();
		expect(data.question).toBeNull();
	});

	it('resolves against what was DRAWN — a sel this read does not contain opens nothing', async () => {
		// The rule the composition branch already had, now held by every read rather than one.
		const data = await run('?sel=019fffff-ffff-7fff-bfff-ffffffffffff');

		expect(data.selected).toBeNull();
		expect(data.selectedExcerpt).toBeNull();
		expect(readResourceBody).not.toHaveBeenCalled();
	});

	it('and no sel at all reads nothing — the rail costs a reader who did not open it nothing', async () => {
		expect((await run()).selected).toBeNull();
		expect(readResourceBody).not.toHaveBeenCalled();
		expect(readTrail).not.toHaveBeenCalled();
	});
});

describe('the excerpt slot never receives a whole document', () => {
	/**
	 * The live bug that rode on the fix. The load used to write
	 * `md === null || node.resource === null ? md : excerptOf({ ...node.resource, content: md })`,
	 * because `excerptOf` asked for a whole `ResourceView` to read one field. An entry mark HAS no
	 * row, so the moment this read resolved a selection that branch handed the entire markdown body
	 * to a slot sized for a paragraph. It was unreachable only because the one read that resolved
	 * selections happened to always carry a row.
	 */
	it('gives the first paragraph of a body, not the body', async () => {
		const body = `${'alpha '.repeat(400)}\n\nA second paragraph nobody asked for.`;
		readResourceBody.mockResolvedValue(body);

		const excerpt = (await run(`?sel=${NODE}`)).selectedExcerpt as string;

		expect(excerpt.length).toBeLessThanOrEqual(601);
		expect(excerpt).not.toContain('A second paragraph');
		expect(excerpt.endsWith('…')).toBe(true);
	});

	it('says nothing when the body read reports nothing, rather than an empty panel', async () => {
		readResourceBody.mockResolvedValue(null);
		expect((await run(`?sel=${NODE}`)).selectedExcerpt).toBeNull();
	});
});

describe('a failed side-read degrades the rail, never the screen', () => {
	it('a trail that will not read leaves the marks drawn and the history absent', async () => {
		readTrail.mockRejectedValue(new Error('502'));

		const data = await run(`?sel=${NODE}`);

		expect(data.selectedTrail).toBeNull();
		expect(data.selected).toBe(NODE);
		expect((data.model as { nodes: unknown[] }).nodes).toHaveLength(2);
	});
});

/**
 * D2 — the handoff. `from` present means NAVIGATE, and the question stops deciding.
 *
 * §10.3, ruled: *"asking a question and our query composition frame helps set the space, but then
 * you traverse the graph as normal without a question locking you in."*
 */
describe('the three-way split — which read an address gets', () => {
	it('routes a `from` to the traversal, and runs no composition at all', async () => {
		// Before D2 this address ran the composition with the hopped-to node as an explicit seed,
		// which is why a hop re-ran the question and drew the answer under a legend reading "In the
		// places you asked about" over a mark the reader had hopped to.
		const data = await run(`?from=${NODE}`);

		expect(readTraversal).toHaveBeenCalledOnce();
		expect(runComposition).not.toHaveBeenCalled();
		expect(readEntry).not.toHaveBeenCalled();
		expect(data.readout).toBeNull();
	});

	it('still traverses when a question is in the address — `q` no longer decides the answer', async () => {
		// The load-bearing half of §10.3. `q` survives the hop as provenance (§10.2), and a surface
		// that re-ran it on every hop would be the grounding locking the reader in.
		await run(`?q=what+am+I+working+on&from=${NODE}`);

		expect(readTraversal).toHaveBeenCalledOnce();
		expect(runComposition).not.toHaveBeenCalled();
	});

	it('carries the question through as provenance rather than dropping or re-asking it', async () => {
		const data = await run(`?q=what+am+I+working+on&from=${NODE}`);

		expect(data.question).toBe('what am I working on');
		// No composition ran, so there is no reasoning to report — and the panel that renders from
		// `question` is provenance, not an explanation of these marks.
		expect(data.readout).toBeNull();
	});

	it('walks the depth the address names, clamped to what the service will actually walk', async () => {
		await run(`?from=${NODE}&depth=9`);

		expect(readTraversal).toHaveBeenCalledWith('tok', [NODE], 3);
	});

	it('walks ONE hop when the address names no depth, and says so in one place', async () => {
		// Resolved in the load rather than left to the service's `unwrap_or(1)`, because the bound
		// line has to report the depth that actually ran.
		await run(`?from=${NODE}`);

		expect(readTraversal).toHaveBeenCalledWith('tok', [NODE], 1);
		expect((await run(`?from=${NODE}`)).bound).toMatchObject({ traversed: { depth: 1 } });
	});

	it('an address with neither still gets the ENTRY read', async () => {
		await run();

		expect(readEntry).toHaveBeenCalledOnce();
		expect(readTraversal).not.toHaveBeenCalled();
		expect(runComposition).not.toHaveBeenCalled();
	});

	it('a question with no `from` still gets the COMPOSITION', async () => {
		// Asserted on the call rather than the result: the mock rejects, which is what makes every
		// other test in this block a real guard rather than a hopeful one.
		await expect(run('?q=what+am+I+working+on')).rejects.toThrow();

		expect(runComposition).toHaveBeenCalledOnce();
		expect(readTraversal).not.toHaveBeenCalled();
		expect(readEntry).not.toHaveBeenCalled();
	});

	it('resolves a selection on a traversal too, because no read may decide that for itself', async () => {
		// The defect #744 closed was one branch forgetting this. A third read was already designed
		// when that fix landed, and `GraphRead` is why this one could not forget it.
		const data = await run(`?from=other-node&sel=${NODE}`);

		expect(data.selected).toBe(NODE);
		expect(readResourceBody).toHaveBeenCalledWith('tok', NODE);
	});

	it('declares its bounds from the marks it drew, not from the seeds it asked for', async () => {
		// A seed the reader cannot read is not returned, so `from` counts what is on SCREEN. The
		// walked fixture contains `NODE`, so hopping from it counts one; hopping from something the
		// response does not contain counts none, and the line says so rather than going quiet.
		expect((await run(`?from=${NODE}`)).bound).toMatchObject({
			traversed: { drawn: 2, from: 1, depth: 1 },
		});
		expect((await run('?from=019fffff-ffff-7fff-bfff-ffffffffffff')).bound).toMatchObject({
			traversed: { drawn: 2, from: 0, depth: 1 },
		});
	});
});
