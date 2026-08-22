// page.server.test.ts — the analysis load, which had no test at all until this file.
//
// It exists for spec §6's cheap route-level guard and nothing else: **the place is a value, the
// measurements are a promise.** Every other test on this route renders `AnalysisPage` against a
// fixture, so all of them pass whether or not this load ever hands back an unsettled promise — and
// the regression most likely to actually happen is *someone adds an `await` and quietly restores
// blocking*.
//
// `vi.mock` over `$lib/server/*` follows the idiom the graph route's `page.server.test.ts`
// established one directory up: module-scope `vi.fn()`s, a `vi.mock` factory forwarding to them,
// then a dynamic `import` of the load so the mocks are installed first.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CogmapRegionMetricsRow, CogmapRegionRow } from '$lib/types/generated/cognitive_maps';

const readAnchorSources = vi.fn();
const readAnchorAnalysis = vi.fn();

vi.mock('$lib/server/graph-query', () => ({
	readAnchorSources: (...a: unknown[]) => readAnchorSources(...a),
	readAnchorAnalysis: (...a: unknown[]) => readAnchorAnalysis(...a),
}));

const { load } = await import('./+page.server');

/** One context the reader can see, so a named place resolves and the measured branch is reachable. */
const CONTEXTS = [
	{ id: 'ctx-1', owner_ref: '@me', slug: 'temper', resource_count: 42 },
] as unknown as Awaited<ReturnType<typeof readAnchorSources>>[0];

const SHAPE = [
	{
		region_id: 'r-1',
		lens_id: 'l-1',
		label: 'A grouping',
		member_count: 3,
		salience: 1.5,
	},
] as unknown as CogmapRegionRow[];

const METRICS = [
	{
		region_id: 'r-1',
		lens_id: 'l-1',
		centrality: 2.5,
		content_cohesion: null,
		internal_tension: 0,
		reference_standing: 1,
		telos_alignment: null,
	},
] as unknown as CogmapRegionMetricsRow[];

const run = (search = '') =>
	(load as (e: unknown) => Promise<Record<string, unknown>>)({
		locals: { accessToken: 'tok' },
		params: { owner: '@me' },
		url: new URL(`https://temperkb.io/graph/@me/analysis${search}`),
	});

beforeEach(() => {
	vi.clearAllMocks();
	readAnchorSources.mockResolvedValue([CONTEXTS, []]);
	readAnchorAnalysis.mockResolvedValue({
		shape: SHAPE,
		metrics: METRICS,
		analytics: null,
		telos: null,
	});
});

describe('the door names the place it is measuring before any measurement arrives', () => {
	/**
	 * C1 for this route, stated at the one place it can actually regress. The read never settles, so
	 * a load that waits for it never returns — the assertion cannot be satisfied by a fast read.
	 */
	it('C1: hands back the place as a value with the measurements still in flight', async () => {
		readAnchorAnalysis.mockReturnValue(new Promise(() => {})); // never settles

		const data = await run('?in=ctx:@me/temper');

		// The scaffold: what the page needs to say WHICH place it is measuring.
		expect(data.place).toEqual({ kind: 'context', ref: '@me/temper', title: '@me/temper' });
		expect(data.place).not.toBeInstanceOf(Promise);
		expect(data.alsoNamed).not.toBeInstanceOf(Promise);
		expect(data.choices).not.toBeInstanceOf(Promise);
		expect(data.refusal).toBeNull();

		// The measured payload, all three fields of it.
		expect(data.regions).toBeInstanceOf(Promise);
		expect(data.metricsAvailable).toBeInstanceOf(Promise);
		expect(data.map).toBeInstanceOf(Promise);
	});

	it('and the measurements are one read, so the three fields cannot disagree', async () => {
		const data = await run('?in=ctx:@me/temper');

		expect(readAnchorAnalysis).toHaveBeenCalledOnce();
		expect(await data.regions).toHaveLength(1);
		expect(await data.metricsAvailable).toBe(true);
	});

	/**
	 * Spec §5.2, on this route. A measurements read that will not answer must reach the page **as a
	 * failure**: resolving to `[]` would say *this place has no groupings*, which is a claim about
	 * the reader's material that nothing verified.
	 */
	it('a measurements read that fails reaches the page as a failure, never as an empty place', async () => {
		readAnchorAnalysis.mockRejectedValue(new Error('503'));

		const data = await run('?in=ctx:@me/temper');

		expect(data.place).not.toBeNull();
		await expect(data.regions).rejects.toThrow('503');
		await expect(data.map).rejects.toThrow('503');
	});
});

describe('the branches that run no read at all', () => {
	/**
	 * The other half of the same contract. On the index and the refusal **nothing is read**, so the
	 * three fields resolve outright — a fact about the branch rather than about a read. They stay
	 * promises because an outer `null` would add a state to fields that already carry their own.
	 */
	it('the index measures nothing, and reads nothing to find that out', async () => {
		const data = await run();

		expect(readAnchorAnalysis).not.toHaveBeenCalled();
		expect(data.place).toBeNull();
		expect(data.refusal).toBeNull();
		expect(data.choices).toHaveLength(1);
		expect(await data.regions).toEqual([]);
		expect(await data.map).toBeNull();
	});

	it('a refusal is the answer rather than a delay, and no read runs behind it', async () => {
		const data = await run('?in=ctx:@me/not-a-place');

		expect(readAnchorAnalysis).not.toHaveBeenCalled();
		expect(data.refusal).toEqual({ kind: 'no-place-resolved', named: 1 });
		expect(data.refusal).not.toBeInstanceOf(Promise);
		expect(await data.regions).toEqual([]);
	});

	it('nothing readable at all is its own refusal, and still reads nothing', async () => {
		readAnchorSources.mockResolvedValue([[], []]);

		const data = await run();

		expect(readAnchorAnalysis).not.toHaveBeenCalled();
		expect(data.refusal).toEqual({ kind: 'nothing-to-analyse' });
	});
});
