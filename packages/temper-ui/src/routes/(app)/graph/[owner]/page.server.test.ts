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
 *
 * `[amended — 2026-08-21]` Since the read streams, that rejection no longer comes out of `load` — it
 * comes out of `data.model`, `data.bound` and `data.readout`. So the loud failure is now a test that
 * awaits one of those, or one of the `mock.calls` assertions; the guard is the same, one field down.
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

		expect(await data.selected).toBe(NODE);
		expect(readResourceBody).toHaveBeenCalledWith('tok', NODE);
		expect(readTrail).toHaveBeenCalledWith('tok', 'node', NODE);
	});

	it('is still the ENTRY read — no composition runs behind the rail', async () => {
		// The mocked `runComposition` rejects, so taking that branch fails rather than quietly
		// answering. Guards the test above from passing on the wrong path.
		const data = await run(`?sel=${NODE}`);

		expect(readEntry).toHaveBeenCalledOnce();
		expect(await data.readout).toBeNull();
		expect(data.question).toBeNull();
	});

	/**
	 * `[amended — 2026-08-21]` The claim is unchanged and the spelling of one half of it is not.
	 *
	 * `selectedExcerpt`'s outer `null` means *no read was started*, and that used to cover this case
	 * too, because the selection was resolved against a settled model. It is now resolved in a
	 * `.then()`, so whether a named `sel` was drawn is knowable only after the model answers — and
	 * the outer null, which the load must spell before it knows, cannot say it any more.
	 *
	 * What must NOT happen is a value standing in for the read: resolving to `null` would say *this
	 * resource has no body*, a claim about the reader's material that no read verified (spec §5.2).
	 * So it rejects, saying only that nothing was read. Nothing consumes it — the rail is gated on
	 * `selected`, which is null — and the load-bearing halves are the two below it: no read ran.
	 */
	it('resolves against what was DRAWN — a sel this read does not contain opens nothing', async () => {
		// The rule the composition branch already had, now held by every read rather than one.
		const data = await run('?sel=019fffff-ffff-7fff-bfff-ffffffffffff');

		expect(await data.selected).toBeNull();
		await expect(data.selectedExcerpt).rejects.toThrow(/not in the answer/);
		expect(readResourceBody).not.toHaveBeenCalled();
		expect(readTrail).not.toHaveBeenCalled();
	});

	it('and no sel at all reads nothing — the rail costs a reader who did not open it nothing', async () => {
		const data = await run();

		expect(await data.selected).toBeNull();
		// Knowable from the ADDRESS, so it is still spelled as the outer null rather than chained
		// off the model: a reader who opened no rail waits for nothing.
		expect(data.selectedExcerpt).toBeNull();
		expect(data.selectedTrail).toBeNull();
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

		// Streamed now, so the load hands back the promise and the awaiting happens here.
		const excerpt = (await (await run(`?sel=${NODE}`)).selectedExcerpt) as string;

		expect(excerpt.length).toBeLessThanOrEqual(601);
		expect(excerpt).not.toContain('A second paragraph');
		expect(excerpt.endsWith('…')).toBe(true);
	});

	it('says nothing when the body read reports nothing, rather than an empty panel', async () => {
		// A resource with no body — the read ANSWERED, and `null` is its answer. Distinct from the
		// read failing, which rejects; see the block below.
		readResourceBody.mockResolvedValue(null);
		expect(await (await run(`?sel=${NODE}`)).selectedExcerpt).toBeNull();
	});
});

describe('a failed side-read degrades the rail, never the screen', () => {
	/**
	 * `[amended — 2026-08-21, spec §5.2]` This test used to assert `expect(data.selectedTrail)
	 * .toBeNull()`, and **that assertion witnessed the defect** rather than a contract: degrading a
	 * failed read to `null` made it indistinguishable from a resource with genuinely no history,
	 * which is the conflation spec §5.1 recorded against this very panel.
	 *
	 * The claim around it is right and stays — *a failed side-read must never take down a screen
	 * whose marks are all drawn*. Only its target changed. The rejection now travels to the rail,
	 * which renders it as a named failure, so what this test witnesses is that the failure **is
	 * still a failure** by the time the page has it, and that the drawn screen is untouched.
	 */
	it('a trail that will not read reaches the rail as a failure, with the marks still drawn', async () => {
		readTrail.mockRejectedValue(new Error('502'));

		const data = await run(`?sel=${NODE}`);

		await expect(data.selectedTrail).rejects.toThrow('502');
		expect(await data.selected).toBe(NODE);
		expect((await (data.model as Promise<{ nodes: unknown[] }>)).nodes).toHaveLength(2);
	});

	it('a body that will not read is a rejection, never an empty excerpt', async () => {
		readResourceBody.mockRejectedValue(new Error('503'));

		const data = await run(`?sel=${NODE}`);

		await expect(data.selectedExcerpt).rejects.toThrow('503');
		expect(await data.selected).toBe(NODE);
	});

	/**
	 * Spec §6's cheap route-level guard, and it catches the regression most likely to actually
	 * happen: *someone adds an `await` and quietly restores blocking*. Nothing else in this file
	 * would notice.
	 */
	it('hands back promises the page has not waited for', async () => {
		readResourceBody.mockReturnValue(new Promise(() => {}));
		readTrail.mockReturnValue(new Promise(() => {}));

		const data = await run(`?sel=${NODE}`);

		expect(await data.selected).toBe(NODE);
		expect(data.selectedExcerpt).toBeInstanceOf(Promise);
		expect(data.selectedTrail).toBeInstanceOf(Promise);
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
		expect(await data.readout).toBeNull();
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
		expect(await data.readout).toBeNull();
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
		expect(await (await run(`?from=${NODE}`)).bound).toMatchObject({ traversed: { depth: 1 } });
	});

	it('an address with neither still gets the ENTRY read', async () => {
		await run();

		expect(readEntry).toHaveBeenCalledOnce();
		expect(readTraversal).not.toHaveBeenCalled();
		expect(runComposition).not.toHaveBeenCalled();
	});

	/**
	 * `[amended — 2026-08-21]` The claim is unchanged; where the rejection surfaces is not.
	 *
	 * The composition read is streamed now, so **the load no longer rejects when that read does** —
	 * the page frame is drawn either way, and the failure travels on the fields to the region that
	 * renders it. That is the change this task is for, so the assertion moved onto the field rather
	 * than being dropped: the branch was taken, and the failure still reaches the page as a failure.
	 */
	it('a question with no `from` still gets the COMPOSITION', async () => {
		// Asserted on the call rather than the result: the mock rejects, which is what makes every
		// other test in this block a real guard rather than a hopeful one.
		const data = await run('?q=what+am+I+working+on');

		expect(runComposition).toHaveBeenCalledOnce();
		expect(readTraversal).not.toHaveBeenCalled();
		expect(readEntry).not.toHaveBeenCalled();
		await expect(data.model).rejects.toThrow('this read must not run a composition');
	});

	it('resolves a selection on a traversal too, because no read may decide that for itself', async () => {
		// The defect #744 closed was one branch forgetting this. A third read was already designed
		// when that fix landed, and `GraphRead` is why this one could not forget it.
		const data = await run(`?from=other-node&sel=${NODE}`);

		expect(await data.selected).toBe(NODE);
		expect(readResourceBody).toHaveBeenCalledWith('tok', NODE);
	});

	/**
	 * Spec §6's route-level guard, and C1 stated at the one place it can actually regress: *someone
	 * adds an `await` and quietly restores blocking*. Every other test in this file would stay
	 * green, because they all await the fields anyway.
	 *
	 * The read never settles, so a load that waits for it never returns — the assertion cannot be
	 * satisfied by a fast read.
	 */
	it('C1: returns the page scaffold with the model still in flight', async () => {
		readEntry.mockReturnValue(new Promise(() => {})); // never settles

		const data = await run();

		expect(data.model).toBeInstanceOf(Promise);
		expect(data.question).not.toBeInstanceOf(Promise); // the scaffold is a value
	});

	/**
	 * The other half of the same contract, and the load-bearing one: **a refusal is the answer, not
	 * a delay.** It is decided from the address and from what the reader can see, above every read,
	 * so it renders with the page chrome rather than behind a loading marker.
	 */
	it('a refusal is settled, and no read runs behind it', async () => {
		const data = await run('?in=ctx:@me/not-a-place');

		expect(data.refusal).toEqual({ kind: 'no-place-resolved', named: 1 });
		expect(data.refusal).not.toBeInstanceOf(Promise);
		expect(readEntry).not.toHaveBeenCalled();
		expect(readTraversal).not.toHaveBeenCalled();
		expect(runComposition).not.toHaveBeenCalled();
	});

	/**
	 * Rung 2 is the OTHER kind of refusal, and the split between the two is what this pins.
	 *
	 * `eligible === 0` is a number the entry read reports, so this verdict cannot precede that read
	 * — it is about the answer that came back, not about the address the reader arrived on. It
	 * therefore streams, and `refusal` stays null: putting it there would have meant awaiting the
	 * entry read to decide the page frame, which is the blocking this route was changed to stop.
	 */
	it('rung 2 arrives with the answer, and `refusal` says nothing about it', async () => {
		readEntry.mockResolvedValue({
			...ENTRY,
			nodes: [],
			edges: [],
			bounds: { drawn: 0, eligible: 0, in_scope: 42, truncated: false },
		});

		const data = await run();

		expect(data.refusal).toBeNull();
		expect(await data.tooLittleStructure).toEqual({
			kind: 'too-little-structure',
			inScope: 42,
		});
	});

	it('and an answer with structure reaches no such verdict', async () => {
		expect(await (await run()).tooLittleStructure).toBeNull();
	});

	it('declares its bounds from the marks it drew, not from the seeds it asked for', async () => {
		// A seed the reader cannot read is not returned, so `from` counts what is on SCREEN. The
		// walked fixture contains `NODE`, so hopping from it counts one; hopping from something the
		// response does not contain counts none, and the line says so rather than going quiet.
		expect(await (await run(`?from=${NODE}`)).bound).toMatchObject({
			traversed: { drawn: 2, from: 1, depth: 1 },
		});
		expect(await (await run('?from=019fffff-ffff-7fff-bfff-ffffffffffff')).bound).toMatchObject({
			traversed: { drawn: 2, from: 0, depth: 1 },
		});
	});
});
