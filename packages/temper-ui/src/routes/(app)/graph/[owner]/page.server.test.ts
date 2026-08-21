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

vi.mock('$lib/server/graph-query', () => ({
	readAnchorSources: (...a: unknown[]) => readAnchorSources(...a),
	readEntry: (...a: unknown[]) => readEntry(...a),
	readResourceBody: (...a: unknown[]) => readResourceBody(...a),
	// Present so the module shape is honest. The entry path calls neither; a test that reached
	// them would fail loudly rather than silently taking a composition branch.
	runComposition: () => Promise.reject(new Error('the entry read must not run a composition')),
	readAnchorRegions: () => Promise.reject(new Error('the entry read discloses no regions')),
	readSeedResources: () => Promise.reject(new Error('an unaddressed entry names no seeds')),
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
