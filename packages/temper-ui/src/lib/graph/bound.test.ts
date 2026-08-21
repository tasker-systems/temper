import { describe, expect, test } from 'vitest';
import type { Extent, QueryResponse, StageResult } from '$lib/types/generated/query';
import { declareBounds, renderBoundLine } from './bound';
import { type Anchor, buildGraphPlan } from './composition';

const anchor = (i: number, n: number): Anchor => ({
	kind: 'context',
	id: `00000000-0000-0000-0000-${String(i).padStart(12, '0')}`,
	ref: `@owner/ctx-${String(i).padStart(3, '0')}`,
	resourceCount: n,
});

/** A returned stage carrying `rows` resource hits. Only the fields the declaration reads are real. */
const stage = (opts: {
	act: string;
	rows: number;
	extent?: Extent;
	regionsApplied?: number;
}): StageResult =>
	({
		act: opts.act,
		disposition: 'answered',
		refusal: null,
		orders_by: null,
		produced: {
			produced: 'resources',
			hits: Array.from({ length: opts.rows }, () => ({}) as never),
		},
		extent: opts.extent ?? { extent: 'complete' },
		total: null,
		terms_applied:
			opts.regionsApplied === undefined ? {} : { regions: BigInt(opts.regionsApplied) },
		narrowed_by: [],
		disclosed_regions: [],
		input_ids: 0n,
		input_unusable: 0n,
	}) as unknown as StageResult;

const response = (returned: Record<string, StageResult>): QueryResponse =>
	({ returned, trace: { stages: [] } }) as unknown as QueryResponse;

/** Two anchors, a question: two survey arms plus the walk. */
const questionPlan = (available = 2) => {
	const outcome = buildGraphPlan({
		anchors: Array.from({ length: available }, (_, i) => anchor(i, 100 - i)),
		question: 'what am I working on',
		seeds: null,
	});
	if (!outcome.ok) throw new Error('expected a plan');
	return outcome.plan;
};

const contextPlan = (asked = 1, available = asked) => {
	const outcome = buildGraphPlan({
		anchors: Array.from({ length: asked }, (_, i) => anchor(i, 100 - i)),
		question: null,
		seeds: null,
		available,
	});
	if (!outcome.ok) throw new Error('expected a plan');
	return outcome.plan;
};

describe('the places axis — the one the client enumerates itself', () => {
	test('declares what was asked against what was available', () => {
		const plan = questionPlan(40);
		const d = declareBounds(response({}), plan);

		expect(d.places).toEqual({ asked: 24, available: 40 });
	});

	test('renders both halves even when nothing was dropped', () => {
		const plan = questionPlan(2);
		const line = renderBoundLine(declareBounds(response({}), plan));

		expect(line).toContain('2 of 2 places');
	});

	test('is singular for one place', () => {
		const line = renderBoundLine(declareBounds(response({}), contextPlan()));

		expect(line).toContain('1 of 1 place');
		expect(line).not.toContain('1 place s');
	});
});

describe('the groupings axis', () => {
	test('renders the APPLIED width, not the width the plan asked for', () => {
		const plan = questionPlan();
		// The plan named 3; the server reports it ran 2. The declaration must say 2.
		const r = response({
			[plan.surveyStages[0]]: stage({ act: 'survey', rows: 5, regionsApplied: 2 }),
			[plan.surveyStages[1]]: stage({ act: 'survey', rows: 5, regionsApplied: 2 }),
			[plan.walkStage]: stage({ act: 'follow-from', rows: 10 }),
		});

		const d = declareBounds(r, plan);

		expect(d.groupings).toEqual({ applicable: true, applied: 2 });
		expect(renderBoundLine(d)).toContain('2 groupings');
		expect(renderBoundLine(d)).not.toContain('3 groupings');
	});

	test('says NOT REPORTED when the response disclosed no width', () => {
		const plan = questionPlan();
		const r = response({
			[plan.surveyStages[0]]: stage({ act: 'survey', rows: 5 }),
			[plan.surveyStages[1]]: stage({ act: 'survey', rows: 5 }),
			[plan.walkStage]: stage({ act: 'follow-from', rows: 10 }),
		});

		const d = declareBounds(r, plan);

		expect(d.groupings).toEqual({ applicable: true, applied: null });
		expect(renderBoundLine(d)).toContain('groupings not reported');
	});

	test('says NOT APPLICABLE for an entry that ran no funnel at all', () => {
		const d = declareBounds(response({}), contextPlan());

		expect(d.groupings).toEqual({ applicable: false });
		expect(renderBoundLine(d)).toContain('groupings not applicable');
	});

	test('a missing axis, an unreported one and a real width all render differently', () => {
		// Held identical on EVERY other axis on purpose. Comparing two whole entries would pass on
		// their other differences — the places count, the presence of the funnel arm — while the
		// groupings phrases were collapsed into one, which is an assertion that cannot observe the
		// thing it names.
		const base = {
			places: { asked: 1, available: 1 },
			inYourPlaces: null,
			fromYourPlaces: null,
			followedOn: { rows: 3, extent: { extent: 'complete' } as Extent },
			orientation: null,
			traversed: null,
		};

		const lines = [
			renderBoundLine({ ...base, groupings: { applicable: false } }),
			renderBoundLine({ ...base, groupings: { applicable: true, applied: null } }),
			renderBoundLine({ ...base, groupings: { applicable: true, applied: 3 } }),
		];

		expect(new Set(lines).size).toBe(3);
	});
});

describe('the two returned arms are declared separately', () => {
	test('the survey arm carries a count and makes no remainder claim', () => {
		const plan = questionPlan();
		const r = response({
			[plan.surveyStages[0]]: stage({
				act: 'survey',
				rows: 18,
				regionsApplied: 3,
				extent: { extent: 'indeterminate', reason: 'a region funnel produces its set' },
			}),
			[plan.surveyStages[1]]: stage({
				act: 'survey',
				rows: 13,
				regionsApplied: 3,
				extent: { extent: 'indeterminate', reason: 'a region funnel produces its set' },
			}),
			[plan.walkStage]: stage({ act: 'follow-from', rows: 50, extent: { extent: 'partial' } }),
		});

		const d = declareBounds(r, plan);

		expect(d.fromYourPlaces).toBe(31);
		const line = renderBoundLine(d);
		expect(line).toContain('31 from your places');
		// The walk's claim must not be attached to the arm that cannot make one.
		expect(line).toBe(
			'Showing 31 from your places · 50 followed on · more exist · 3 groupings per place · 2 of 2 places',
		);
	});

	test("the walk's extent is the walk's alone and is not diluted by the other arm", () => {
		const plan = questionPlan();
		const r = response({
			[plan.surveyStages[0]]: stage({
				act: 'survey',
				rows: 4,
				regionsApplied: 3,
				extent: { extent: 'indeterminate', reason: 'no remainder to report' },
			}),
			[plan.surveyStages[1]]: stage({ act: 'survey', rows: 0, regionsApplied: 3 }),
			[plan.walkStage]: stage({
				act: 'follow-from',
				rows: 12,
				extent: { extent: 'complete' },
			}),
		});

		expect(renderBoundLine(declareBounds(r, plan))).toContain('12 followed on · complete');
	});

	test('a context entry has no survey arm, so that figure is absent rather than zero', () => {
		const plan = contextPlan();
		const r = response({
			[plan.walkStage]: stage({ act: 'follow-from', rows: 50, extent: { extent: 'partial' } }),
		});

		const d = declareBounds(r, plan);

		expect(d.fromYourPlaces).toBeNull();
		expect(renderBoundLine(d)).toBe(
			'Showing 50 followed on · more exist · groupings not applicable · 1 of 1 place',
		);
	});

	test('an indeterminate walk says so rather than claiming completeness', () => {
		const plan = contextPlan();
		const r = response({
			[plan.walkStage]: stage({
				act: 'follow-from',
				rows: 3,
				extent: { extent: 'indeterminate', reason: 'the stage refused' },
			}),
		});

		expect(renderBoundLine(declareBounds(r, plan))).toContain(
			'3 followed on · completeness not reported',
		);
	});

	test('completeness is TOLD, not inferred from the absence of a warning', () => {
		const plan = contextPlan();
		const r = response({
			[plan.walkStage]: stage({ act: 'follow-from', rows: 7, extent: { extent: 'complete' } }),
		});

		expect(renderBoundLine(declareBounds(r, plan))).toContain('complete');
	});
});

describe('no internal vocabulary is load-bearing', () => {
	const forbidden = ['region', 'salience', 'wayfind', 'survey'];

	const everyLine = () => {
		const q = questionPlan(40);
		const c = contextPlan();
		const extents: Extent[] = [
			{ extent: 'complete' },
			{ extent: 'partial' },
			{ extent: 'indeterminate', reason: 'a region funnel produces its own candidate set' },
		];
		const lines: string[] = [];
		for (const extent of extents) {
			for (const regionsApplied of [undefined, 3]) {
				lines.push(
					renderBoundLine(
						declareBounds(
							response({
								[q.surveyStages[0]]: stage({ act: 'survey', rows: 9, regionsApplied }),
								[q.walkStage]: stage({ act: 'follow-from', rows: 50, extent }),
							}),
							q,
						),
					),
					renderBoundLine(
						declareBounds(
							response({
								[c.walkStage]: stage({ act: 'follow-from', rows: 1, extent }),
							}),
							c,
						),
					),
				);
			}
		}
		return lines;
	};

	test.each(forbidden)('the rendered line never contains %s', (word) => {
		for (const line of everyLine()) {
			expect(line.toLowerCase()).not.toContain(word);
		}
	});

	test('an indeterminate reason from the server is never echoed verbatim', () => {
		// The server's own reason text says "region funnel". Echoing it would leak the vocabulary
		// through a field the surface does not control.
		const plan = contextPlan();
		const r = response({
			[plan.walkStage]: stage({
				act: 'follow-from',
				rows: 2,
				extent: { extent: 'indeterminate', reason: 'a region funnel produces its set' },
			}),
		});

		expect(renderBoundLine(declareBounds(r, plan))).not.toContain('funnel');
	});
});

/**
 * The seed arm — the only axis on this screen with a true denominator.
 *
 * A no-question entry's own rows do not come from the composition at all: `follow-from` walks at
 * least one hop, so the seeds are not in the walked arm. They come from the list read, which
 * reports `total` — *"every row the filters admit, before `limit`/`offset`"*.
 */
describe('the seed arm', () => {
	const seeds = { shown: 200, total: 2066, truncated: true };

	test('states its denominator, because unlike every other arm it has one', () => {
		const d = declareBounds(response({}), contextPlan(), seeds);

		expect(d.inYourPlaces).toEqual(seeds);
		expect(renderBoundLine(d)).toContain('200 of 2066 in this place');
	});

	test('names one place and many places differently', () => {
		const one = declareBounds(response({}), contextPlan(), seeds);
		expect(renderBoundLine(one)).toContain('in this place');

		const many = declareBounds(response({}), contextPlan(4, 4), seeds);
		expect(renderBoundLine(many)).toContain('across your places');
	});

	test('a complete arm and a truncated one do not render alike', () => {
		const complete = renderBoundLine(
			declareBounds(response({}), contextPlan(), { shown: 12, total: 12, truncated: false }),
		);
		const partial = renderBoundLine(
			declareBounds(response({}), contextPlan(), { shown: 12, total: 900, truncated: true }),
		);

		expect(complete).not.toBe(partial);
		expect(complete).toContain('12 of 12');
		expect(partial).toContain('12 of 900');
	});

	test('an entry with no seed arm says nothing about one — absence, not zero', () => {
		const line = renderBoundLine(declareBounds(response({}), contextPlan(), null));

		expect(line).not.toContain('of 0');
		expect(line).not.toContain('in this place');
		expect(line).not.toContain('across your places');
	});

	test('the arm is omitted by DEFAULT, so a caller that has none cannot accidentally claim one', () => {
		expect(declareBounds(response({}), contextPlan()).inYourPlaces).toBeNull();
	});
});
