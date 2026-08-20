import { describe, expect, test } from 'vitest';
import { jsonBody } from '$lib/server/json-body';
import type { ActInvocation, CombineNode, StageNode } from '$lib/types/generated/query';
import {
	ANCHOR_CEILING,
	type Anchor,
	buildGraphPlan,
	REGIONS_PER_ANCHOR,
	WALK_LIMIT,
} from './composition';

/** A context anchor with `n` resources. `ref` is what ties break on, so it is distinct per index. */
const ctx = (i: number, n: number): Anchor => ({
	kind: 'context',
	id: `00000000-0000-0000-0000-${String(i).padStart(12, '0')}`,
	ref: `@owner/ctx-${String(i).padStart(3, '0')}`,
	resourceCount: n,
});

const map = (i: number, n: number): Anchor => ({
	kind: 'cogmap',
	id: `11111111-0000-0000-0000-${String(i).padStart(12, '0')}`,
	ref: `map-${String(i).padStart(3, '0')}-11111111-0000-0000-0000-${String(i).padStart(12, '0')}`,
	resourceCount: n,
});

const acts = (stages: StageNode[]): ActInvocation[] =>
	stages.filter((s): s is ActInvocation => 'act' in s);

const combines = (stages: StageNode[]): CombineNode[] =>
	stages.filter((s): s is CombineNode => 'op' in s);

const actsNamed = (stages: StageNode[], act: string) => acts(stages).filter((s) => s.act === act);

/** Every stage name a composition references must resolve, and none may repeat. */
const referentialIntegrity = (stages: StageNode[]) => {
	const names = stages.map((s) => s.name);
	expect(new Set(names).size, 'duplicate stage name').toBe(names.length);
	for (const s of stages) {
		const upstream =
			'op' in s ? s.inputs : s.inputs.flatMap((i) => (i.from === 'upstream' ? [i.stage] : []));
		for (const u of upstream) expect(names, `dangling reference to ${u}`).toContain(u);
	}
};

describe('the anchor-set ceiling', () => {
	test('40 anchors are truncated to the ceiling, and the plan records what it dropped', () => {
		const anchors = Array.from({ length: 40 }, (_, i) => ctx(i, 100 - i));

		const outcome = buildGraphPlan({ anchors, question: 'what am I working on', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		expect(outcome.plan.anchorsAsked).toHaveLength(ANCHOR_CEILING);
		expect(outcome.plan.anchorsAvailable).toBe(40);
	});

	test('the anchors dropped are the emptiest ones, not an arbitrary 24', () => {
		// Shuffled so a builder that merely takes the first 24 in input order fails here.
		const anchors = [ctx(0, 1), ctx(1, 900), ctx(2, 0), ctx(3, 500)];

		const outcome = buildGraphPlan({ anchors, question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		expect(outcome.plan.anchorsAsked.map((a) => a.resourceCount)).toEqual([900, 500, 1, 0]);
	});

	test('anchors with equal counts are ordered by ref, so the plan is reproducible', () => {
		const anchors = [ctx(9, 5), ctx(1, 5), ctx(4, 5)];

		const outcome = buildGraphPlan({ anchors, question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		expect(outcome.plan.anchorsAsked.map((a) => a.ref)).toEqual([
			'@owner/ctx-001',
			'@owner/ctx-004',
			'@owner/ctx-009',
		]);
	});
});

describe('the question entry', () => {
	test('two or more anchors fan out one survey each and pipe through a union', () => {
		const outcome = buildGraphPlan({
			anchors: [ctx(0, 10), ctx(1, 20), map(0, 30)],
			question: 'what am I working on',
			seeds: null,
		});

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const { stages } = outcome.plan.composition;
		expect(actsNamed(stages, 'survey')).toHaveLength(3);
		expect(combines(stages)).toHaveLength(1);
		expect(combines(stages)[0].op).toBe('union');
		expect(actsNamed(stages, 'follow-from')).toHaveLength(1);
		referentialIntegrity(stages);
	});

	test('a single anchor emits NO union, because a one-input combinator is refused', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 10)], question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const { stages } = outcome.plan.composition;
		expect(actsNamed(stages, 'survey')).toHaveLength(1);
		expect(combines(stages)).toHaveLength(0);
		// The walk reads the survey directly.
		const walk = actsNamed(stages, 'follow-from')[0];
		expect(walk.inputs).toEqual([
			{ from: 'upstream', as: 'seed', stage: actsNamed(stages, 'survey')[0].name },
		]);
		referentialIntegrity(stages);
	});

	test('zero anchors ask the act that needs no organization at all', () => {
		const outcome = buildGraphPlan({ anchors: [], question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const { stages } = outcome.plan.composition;
		expect(actsNamed(stages, 'survey')).toHaveLength(0);
		expect(actsNamed(stages, 'find-about-anywhere')).toHaveLength(1);
		// No anchor means no bound: an empty context/cogmap IdSet is refused outright.
		expect(actsNamed(stages, 'find-about-anywhere')[0].inputs).toEqual([]);
		referentialIntegrity(stages);
	});

	test('every survey names its regions term, or the applied width is never disclosed', () => {
		const outcome = buildGraphPlan({
			anchors: [ctx(0, 10), ctx(1, 20)],
			question: 'q',
			seeds: null,
		});

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		for (const s of actsNamed(outcome.plan.composition.stages, 'survey')) {
			expect(s.terms.regions).toBe(BigInt(REGIONS_PER_ANCHOR));
		}
	});

	test('a survey binds exactly one anchor id, under its own kind', () => {
		const outcome = buildGraphPlan({
			anchors: [ctx(0, 10), map(0, 30)],
			question: 'q',
			seeds: null,
		});

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const kinds = actsNamed(outcome.plan.composition.stages, 'survey').map((s) => {
			const input = s.inputs[0];
			if (input.from !== 'caller') throw new Error('a survey binds a caller-supplied anchor');
			expect(input.as).toBe('bound');
			expect(input.ids.ids).toHaveLength(1);
			return input.ids.kind;
		});
		expect(kinds.sort()).toEqual(['cogmap', 'context']);
	});

	test('the surveys and the walk are returned as separate arms', () => {
		const outcome = buildGraphPlan({
			anchors: [ctx(0, 10), ctx(1, 20)],
			question: 'q',
			seeds: null,
		});

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const returns = outcome.plan.composition.outcome.returns.map((r) => r.stage);
		expect(returns).toHaveLength(3);
		expect(returns).toContain(outcome.plan.walkStage);
		for (const s of outcome.plan.surveyStages) expect(returns).toContain(s);
	});

	test('the walk names its own ceiling', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 10)], question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const walk = actsNamed(outcome.plan.composition.stages, 'follow-from')[0];
		expect(walk.terms.limit).toBe(BigInt(WALK_LIMIT));
	});
});

describe('cross-kind reach', () => {
	test('a union spans a context anchor and a cogmap anchor in one plan', () => {
		const outcome = buildGraphPlan({
			anchors: [ctx(0, 10), map(0, 30)],
			question: 'q',
			seeds: null,
		});

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const { stages } = outcome.plan.composition;
		const union = combines(stages)[0];
		expect(union.inputs).toHaveLength(2);

		const boundKindOf = (stageName: string) => {
			const stage = acts(stages).find((s) => s.name === stageName);
			const input = stage?.inputs[0];
			if (input?.from !== 'caller') throw new Error('expected a caller-bound anchor');
			return input.ids.kind;
		};
		expect(union.inputs.map(boundKindOf).sort()).toEqual(['cogmap', 'context']);
	});
});

describe('the no-question entry', () => {
	test('a context with no question selects everything in it, and returns only the walk', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 10)], question: null, seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const { stages } = outcome.plan.composition;
		expect(actsNamed(stages, 'find-resources-with')).toHaveLength(1);
		expect(actsNamed(stages, 'survey')).toHaveLength(0);
		// A selection stage is refused in `returns` — it produces ids, not rows.
		expect(outcome.plan.composition.outcome.returns.map((r) => r.stage)).toEqual([
			outcome.plan.walkStage,
		]);
		expect(outcome.plan.surveyStages).toEqual([]);
		referentialIntegrity(stages);
	});

	test('the selection carries no intention, because it asks nothing', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 10)], question: null, seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const sel = actsNamed(outcome.plan.composition.stages, 'find-resources-with')[0];
		expect(sel.intention).toBeNull();
		expect(sel.terms).toEqual({});
	});

	test('no anchors and no question is nothing to ask, not an empty composition', () => {
		const outcome = buildGraphPlan({ anchors: [], question: null, seeds: null });

		expect(outcome.ok).toBe(false);
		if (outcome.ok) return;
		expect(outcome.reason).toBe('nothing-to-ask');
	});
});

describe('an anchor with no derived structure behind it', () => {
	test('a zero-resource anchor still produces a valid plan', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 0)], question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		expect(actsNamed(outcome.plan.composition.stages, 'survey')).toHaveLength(1);
		referentialIntegrity(outcome.plan.composition.stages);
	});

	test('a reader with no question and a bare context touches no funnel at all', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 0)], question: null, seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		expect(actsNamed(outcome.plan.composition.stages, 'survey')).toEqual([]);
	});
});

describe('explicit seeds', () => {
	test('seeds replace the upstream stage as what the walk grows from', () => {
		const seeds = ['aaaaaaaa-0000-0000-0000-000000000001'];

		const outcome = buildGraphPlan({ anchors: [ctx(0, 10)], question: 'q', seeds });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const walk = acts(outcome.plan.composition.stages).find((s) => s.act === 'follow-from');
		expect(walk?.inputs).toEqual([
			{ from: 'caller', as: 'seed', ids: { kind: 'resource', provenance: null, ids: seeds } },
		]);
	});
});

/**
 * The class of defect a fixture cannot see.
 *
 * Every test above compares the built object to an expected shape, and a comparison never sends
 * anything. Both failures below shipped green under 69 such tests and were found the first time the
 * builder's output met `/api/query` — one refused by the contract before the server ran it, the
 * other unable to leave the process at all.
 */
describe('what only a real request could see', () => {
	test('every survey carries the question — a survey without one is REFUSED, not merely blunt', () => {
		// `temper query --check` (local, no server) on a plan whose surveys carried no intention:
		//   { "expressible": false, "refusals": [
		//       { "stage": "s1", "reason": "missing_intention",
		//         "detail": "this survey act carries no intention, and survey needs a question" } ] }
		// The act's own gated wrapper says why: "Survey requires an intention (query + embedding);
		// without one it collapses into cogmap_read(shape)/context_read(shape), which already serve
		// pure orientation." So an intention-less survey is not a broader survey — it is no plan.
		const question = 'what am I working on';
		const outcome = buildGraphPlan({
			anchors: [ctx(0, 10), ctx(1, 20), map(0, 30)],
			question,
			seeds: null,
		});

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		const surveys = actsNamed(outcome.plan.composition.stages, 'survey');
		expect(surveys).toHaveLength(3);
		for (const s of surveys) {
			expect(s.intention?.query, `survey ${s.name} carries no question`).toBe(question);
		}
	});

	test('the zero-anchor entry carries the question too', () => {
		const outcome = buildGraphPlan({ anchors: [], question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		expect(
			actsNamed(outcome.plan.composition.stages, 'find-about-anywhere')[0].intention?.query,
		).toBe('q');
	});

	test('no act that requires a question is ever emitted without one', () => {
		// Stated over the whole family rather than per entry, so an act added later to the builder
		// is covered by this test on the day it is added rather than on the day someone remembers.
		const REQUIRES_A_QUESTION = ['survey', 'find-about-anywhere', 'find-about-within'];

		for (const anchors of [[], [ctx(0, 10)], [ctx(0, 10), map(0, 5)]]) {
			const outcome = buildGraphPlan({ anchors, question: 'q', seeds: null });
			expect(outcome.ok).toBe(true);
			if (!outcome.ok) return;
			for (const s of acts(outcome.plan.composition.stages)) {
				if (!REQUIRES_A_QUESTION.includes(s.act)) continue;
				expect(s.intention?.query, `${s.act} stage ${s.name} carries no question`).toBe('q');
			}
		}
	});

	test('the composition survives encoding — its terms are bigint, which JSON.stringify refuses', () => {
		const outcome = buildGraphPlan({ anchors: [ctx(0, 10)], question: 'q', seeds: null });

		expect(outcome.ok).toBe(true);
		if (!outcome.ok) return;
		// The builder is right to hold bigint: `Composition.terms` is `{[key in BoundTerm]?: bigint}`
		// in the generated contract. It is the SENDING that has to know, which is why the encoder
		// lives at the request boundary (`lib/server/json-body.ts`) rather than here.
		expect(() => JSON.stringify(outcome.plan.composition)).toThrow();
		expect(JSON.parse(jsonBody(outcome.plan.composition))).toMatchObject({
			stages: expect.arrayContaining([
				expect.objectContaining({ act: 'survey', terms: { regions: REGIONS_PER_ANCHOR } }),
				expect.objectContaining({ act: 'follow-from', terms: { limit: WALK_LIMIT } }),
			]),
		});
	});
});
