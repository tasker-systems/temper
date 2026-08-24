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

describe('the pooled read still drops the envelope, and that is the decision', () => {
	/**
	 * `readAnchorRegions` reads MANY anchors and is called ONLY when regions were disclosed
	 * (`+page.server.ts`), so it never meets an empty answer and has no single cause to report.
	 * Pinned so that "the other door carries it, why not this one" is answered by a failing test
	 * rather than by a comment someone can skim past.
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
});
