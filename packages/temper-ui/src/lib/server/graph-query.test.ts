// graph-query.test.ts — the two shape doors, at the seam where the envelope is unwrapped.
//
// **Why this file exists.** Every other test on this path either mocks `readAnchorAnalysis`
// wholesale (`analysis/page.server.test.ts`) or hands a component an `AnalysisViewData` a test
// built by hand. So the one line that actually reads `emptiness` off the wire had no test at all:
// replacing `emptiness: shape.emptiness` with `emptiness: null` left all 840 tests green AND
// typechecked, because the declared return type admits `null`. That is the door throwing the
// envelope away again — precisely the defect this work exists to close — hiding one layer below
// the layer that got tested.
//
// `./api` is mocked rather than `fetch`, so these pin the unwrap and the path choice without
// standing up an HTTP surface. What they must NOT do is re-test `apiGet`.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Anchor } from '$lib/graph/composition';
import type { AnchorShape } from '$lib/types/generated/cognitive_maps';

const apiGet = vi.fn();

vi.mock('./api', () => ({
	apiGet: (...a: unknown[]) => apiGet(...a),
	apiPost: vi.fn(),
}));

const { readAnchorAnalysis, readAnchorRegions } = await import('./graph-query');

const CONTEXT: Anchor = { kind: 'context', id: 'ctx-1', ref: '@me/temper', resourceCount: 42 };
const COGMAP: Anchor = { kind: 'cogmap', id: 'map-1', ref: 'self-cognition', resourceCount: 9 };

/** An envelope whose row set is EMPTY and which therefore carries a cause. */
const emptyShape = (over: Partial<AnchorShape> = {}): AnchorShape =>
	({
		regions: [],
		population: 7,
		emptiness: 'nothing_visible',
		materialized_at: '2026-08-20T10:00:00Z',
		...over,
	}) as AnchorShape;

const ROW = { region_id: 'r-1', lens_id: 'l-1', label: 'A grouping', member_count: 3, salience: 1 };

/** What `/api/contexts/{id}/analytics` returns: the clock, bare, and nothing beside it. */
const CLOCK = {
	materialized_at: '2026-08-20T10:00:00.000Z',
	latest_touch: '2026-08-21T09:00:00.000Z',
	is_stale: true,
};

/** What `/api/cognitive-maps/{id}/analytics` returns: the same clock, plus the two a map has. */
const ANALYTICS_ROW = { telos_resource_id: 'res-1', staleness: CLOCK, regulation: [] };

beforeEach(() => {
	vi.clearAllMocks();
});

describe('the analysis door carries the cause off the wire', () => {
	/** Everything except `shape` degrades or is skipped, so one mock per path keeps this honest. */
	const routeReads = (shape: AnchorShape) => {
		apiGet.mockImplementation((path: string) => {
			if (path.endsWith('/shape')) return Promise.resolve(shape);
			return Promise.reject(new Error('not under test'));
		});
	};

	it('hands back the cause the envelope carried, not a cause it invented', async () => {
		routeReads(emptyShape({ emptiness: 'never_clustered' }));

		const out = await readAnchorAnalysis('tok', CONTEXT);

		// The assertion mutation 11 was invisible to: the door must READ the field, not default it.
		expect(out.emptiness).toBe('never_clustered');
		expect(out.shape).toEqual([]);
	});

	it('reports no cause when the row set is not empty, because there is none to report', async () => {
		routeReads(emptyShape({ regions: [ROW] as AnchorShape['regions'], emptiness: null }));

		const out = await readAnchorAnalysis('tok', CONTEXT);

		expect(out.emptiness).toBeNull();
		// The unwrap itself: `shape` is the ROWS, not the envelope around them.
		expect(out.shape).toEqual([ROW]);
	});

	/**
	 * The two omissions, pinned as DECISIONS rather than left to look like oversights.
	 *
	 * `population` is dropped because this door passes no lens, so it always equals
	 * `regions.length` — it would print the row count twice under two names. `materialized_at` is
	 * dropped because the page already shows a clock for maps and this one is stamped at the
	 * materialize transaction's start, so it runs systematically early.
	 *
	 * If a later change carries either of them, this test fails and that change is a visible
	 * choice with a reason attached rather than a quiet widening.
	 */
	it('drops population and materialized_at at the door, deliberately', async () => {
		routeReads(emptyShape());

		const out = await readAnchorAnalysis('tok', CONTEXT);

		expect(Object.keys(out).sort()).toEqual([
			'analytics',
			'emptiness',
			'metrics',
			'shape',
			'telos',
		]);
	});

	it('asks the door that matches the anchor kind', async () => {
		routeReads(emptyShape());
		await readAnchorAnalysis('tok', CONTEXT);
		expect(apiGet).toHaveBeenCalledWith('/api/contexts/ctx-1/shape', 'tok');

		vi.clearAllMocks();
		routeReads(emptyShape());
		await readAnchorAnalysis('tok', COGMAP);
		expect(apiGet).toHaveBeenCalledWith('/api/cognitive-maps/map-1/shape', 'tok');
	});

	/**
	 * **Both anchor kinds are asked for the anchor-level readout, and for DIFFERENT shapes.**
	 *
	 * A context was not asked at all until `/api/contexts/{id}/analytics` shipped, and the door
	 * handed it `null` — which the page then spelled as *a context has no map-level readout*. These
	 * two pin the replacement: the door is asked, and what comes back is tagged with the kind that
	 * asked, so nothing downstream has to infer a context from the absence of a charter.
	 *
	 * The SHAPE is the assertion that matters. A context has no charter resource and no regulation
	 * set, so widening this into one row with both of them nullable would spell *nothing found*
	 * about two things that cannot exist.
	 */
	it('asks the context door for the clock, and tags what comes back as a context', async () => {
		apiGet.mockImplementation((path: string) => {
			if (path.endsWith('/shape')) return Promise.resolve(emptyShape());
			if (path === '/api/contexts/ctx-1/analytics') return Promise.resolve(CLOCK);
			return Promise.reject(new Error('not under test'));
		});

		const out = await readAnchorAnalysis('tok', CONTEXT);

		expect(apiGet).toHaveBeenCalledWith('/api/contexts/ctx-1/analytics', 'tok');
		expect(out.analytics).toEqual({ kind: 'context', staleness: CLOCK });
		// No charter to look up, so none is looked up — and none is invented as a null peer field.
		expect(out.telos).toBeNull();
		for (const [path] of apiGet.mock.calls) expect(String(path)).not.toContain('/api/resources/');
	});

	it('tags the cogmap door’s five-field row as a cogmap, and follows its charter', async () => {
		apiGet.mockImplementation((path: string) => {
			if (path.endsWith('/shape')) return Promise.resolve(emptyShape());
			if (path === '/api/cognitive-maps/map-1/analytics') return Promise.resolve(ANALYTICS_ROW);
			if (path === '/api/resources/res-1') return Promise.resolve({ title: 'The charter' });
			return Promise.reject(new Error('not under test'));
		});

		const out = await readAnchorAnalysis('tok', COGMAP);

		expect(apiGet).toHaveBeenCalledWith('/api/cognitive-maps/map-1/analytics', 'tok');
		expect(out.analytics).toEqual({ kind: 'cogmap', ...ANALYTICS_ROW });
		expect(out.telos).toEqual({ title: 'The charter' });
	});

	/** A deny is a 404, and a 404 is `null` — for either kind, and never an error. */
	it('a declined anchor-level read is null rather than a throw, for a context too', async () => {
		routeReads(emptyShape());

		await expect(readAnchorAnalysis('tok', CONTEXT)).resolves.toMatchObject({ analytics: null });
	});

	/**
	 * **No `lens` is passed, and that is definitional rather than a default.** The lens is a
	 * clustering-time parameter; naming one at read time would look up a different set of regions
	 * than the place actually published. It is also what makes `lens_narrowed` unreachable here.
	 */
	it('passes no lens, which is what keeps lens_narrowed unreachable at this door', async () => {
		routeReads(emptyShape());

		await readAnchorAnalysis('tok', CONTEXT);

		for (const [path] of apiGet.mock.calls) expect(String(path)).not.toContain('lens');
	});
});

describe('the pooled read reports no cause, but does read one arm', () => {
	/**
	 * `readAnchorRegions` reads MANY anchors and is called ONLY when regions were disclosed
	 * (`+page.server.ts`), so it never meets an empty answer and has no single cause to REPORT — the
	 * return shape stays `{ rows, complete }`. What it does read is `unreadable_or_absent`, and only
	 * to answer the question `complete` already asked; see the two tests below. Pinned so that "the
	 * other door carries the whole envelope, why not this one" is answered by a failing test rather
	 * than by a comment someone can skim past.
	 */
	it('returns rows and completeness, and no per-anchor cause', async () => {
		apiGet.mockResolvedValue(emptyShape({ regions: [ROW] as AnchorShape['regions'] }));

		const out = await readAnchorRegions('tok', [CONTEXT, COGMAP]);

		expect(Object.keys(out).sort()).toEqual(['complete', 'rows']);
		expect(out.rows).toEqual([ROW, ROW]);
		expect(out.complete).toBe(true);
	});

	/**
	 * A read that did not answer degrades the whole lookup rather than propagating. `complete` is
	 * NOT an `emptiness` arm and must never be collapsed into one: it means *a read did not answer*,
	 * which downgrades unfound ids to `unchecked` rather than to `re-derived`.
	 */
	it('one unreadable anchor makes the gathering incomplete, not the page an error', async () => {
		apiGet
			.mockResolvedValueOnce(emptyShape({ regions: [ROW] as AnchorShape['regions'] }))
			.mockRejectedValueOnce(new Error('403'));

		const out = await readAnchorRegions('tok', [CONTEXT, COGMAP]);

		expect(out.rows).toEqual([ROW]);
		expect(out.complete).toBe(false);
	});

	/**
	 * **The denial that arrives as a success.** A caller who may not read an anchor gets
	 * `unreadable_or_absent` with zero rows on a **200**, never a 403 — the posture that keeps the
	 * shape read from being an existence oracle. Under `every(fulfilled)` that counted as an answer,
	 * so `complete` stayed true and `nameOf` told the reader their grouping had been re-derived on
	 * the strength of a read that disclosed nothing to them.
	 */
	it('a denial delivered as an empty 200 is a non-answer, not a completed read', async () => {
		apiGet
			.mockResolvedValueOnce(emptyShape({ regions: [ROW] as AnchorShape['regions'] }))
			.mockResolvedValueOnce(emptyShape({ emptiness: 'unreadable_or_absent', population: 0 }));

		const out = await readAnchorRegions('tok', [CONTEXT, COGMAP]);

		expect(out.rows).toEqual([ROW]);
		expect(out.complete, 'a 200 that declined to answer is not a completed read').toBe(false);
	});

	/**
	 * The other three arms are ANSWERS, and must keep counting as such — otherwise this really would
	 * collapse `complete` into `emptiness`, and every reader of a legitimately empty anchor would be
	 * told their groupings were merely unchecked.
	 */
	it('an anchor that genuinely holds nothing for this caller still counts as answered', async () => {
		for (const arm of ['never_clustered', 'nothing_visible', 'lens_narrowed'] as const) {
			apiGet.mockReset();
			apiGet.mockResolvedValue(emptyShape({ emptiness: arm }));

			const out = await readAnchorRegions('tok', [CONTEXT]);

			expect(out.complete, `${arm} is an answer about the anchor`).toBe(true);
		}
	});
});
