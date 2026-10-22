import { describe, expect, it } from 'vitest';
import { type GraphOmissions, graphSentence, graphStanding, omissionSentence } from './omission.js';

describe('the omission sentence', () => {
	it('says what an unpaged view leaves out', () => {
		expect(omissionSentence(12, 12, 'in this context')).toBe('All 12 in this context.');
		expect(omissionSentence(12, 3, 'in this context')).toBe(
			'3 of 12 in this context; 9 not shown.',
		);
	});

	it('says a first page of 50 out of 51 is not all of them', () => {
		const page = { offset: 0, size: 50, more: true };
		expect(omissionSentence(51, 50, 'tasks', page)).toBe('1–50 of 51 tasks; 1 after it.');
	});

	it('says what lies either side of a middle page, and only before the last', () => {
		expect(omissionSentence(230, 50, 'tasks', { offset: 50, size: 50, more: true })).toBe(
			'51–100 of 230 tasks; 50 before this page, 130 after it.',
		);
		expect(omissionSentence(230, 30, 'tasks', { offset: 200, size: 50, more: false })).toBe(
			'201–230 of 230 tasks; 200 before this page.',
		);
	});

	it('calls a single whole page all of them', () => {
		expect(omissionSentence(7, 7, 'tasks', { offset: 0, size: 50, more: false })).toBe(
			'All 7 tasks.',
		);
	});
});

describe("a graph view's omission sentences", () => {
	const SCOPE = 'the most-connected in +temper-dev/contrib';

	const walk = (depth: number, cut: GraphOmissions['cut'] = null): GraphOmissions => ({
		arm: { read: 'walk', from: ['seed-1'], depth },
		bounds: null,
		cut,
	});
	const entry = (
		bounds: NonNullable<GraphOmissions['bounds']>,
		cut: GraphOmissions['cut'] = null,
	): GraphOmissions => ({ arm: { read: 'entry', in: ['ctx-1'], k: 130 }, bounds, cut });

	it('says a walk went how deep, and that deeper was not reported — no denominator', () => {
		expect(graphSentence(5, 5, 'reached from Mara within 2 hops', walk(2))).toBe(
			'All 5 reached from Mara within 2 hops; deeper not reported.',
		);
	});

	it('counts a walk cut to the drawing’s bounds, each part only when some were', () => {
		expect(
			graphSentence(600, 200, 'reached from Mara within 3 hops', walk(3, { nodes: 400 })),
		).toBe(
			'200 of 600 reached from Mara within 3 hops; deeper not reported; 400 nodes past the drawing’s bounds not drawn.',
		);
		expect(
			graphSentence(600, 200, 'reached from Mara within 3 hops', walk(3, { nodes: 400, edges: 1 })),
		).toBe(
			'200 of 600 reached from Mara within 3 hops; deeper not reported; 400 nodes and 1 edge past the drawing’s bounds not drawn.',
		);
	});

	it('says an entry read drew of eligible, and names the k it was asked at', () => {
		expect(
			graphSentence(
				130,
				130,
				SCOPE,
				entry({ drawn: 130, eligible: 780, inScope: 780, truncated: false }),
			),
		).toBe(`Drawn 130 of 780 eligible, ${SCOPE} at a draw of 130.`);
		expect(
			graphSentence(
				12,
				12,
				SCOPE,
				entry({ drawn: 12, eligible: 12, inScope: 12, truncated: false }),
			),
		).toBe(`All 12 ${SCOPE} at a draw of 130.`);
	});

	it('counts the in-scope nodes the entry read did not draw for having no connections', () => {
		expect(
			graphSentence(
				12,
				12,
				SCOPE,
				entry({ drawn: 12, eligible: 12, inScope: 40, truncated: false }),
			),
		).toBe(`All 12 ${SCOPE} at a draw of 130; 28 in scope not connected, not drawn.`);
		expect(
			graphSentence(
				130,
				130,
				SCOPE,
				entry({ drawn: 130, eligible: 780, inScope: 3000, truncated: true }, { edges: 20 }),
			),
		).toBe(
			`Drawn 130 of 780 eligible, ${SCOPE} at a draw of 130; 2220 in scope not connected, not drawn; 20 edges past the drawing’s bounds not drawn.`,
		);
	});

	it('composes a view with no read behind it as the plain sentence', () => {
		const none: GraphOmissions = { arm: null, bounds: null, cut: null };
		expect(graphSentence(12, 12, 'around this goal', none)).toBe('All 12 around this goal.');
		expect(graphSentence(12, 3, 'around this goal', none)).toBe(
			'3 of 12 around this goal; 9 not shown.',
		);
	});

	it('names where a figure was read from, standing point and depth or draw', () => {
		const nameOf = (id: string) => (id === 'seed-1' ? 'Mara' : id);
		expect(graphStanding({ read: 'walk', from: ['seed-1'], depth: 2 }, nameOf)).toBe(
			'reached from Mara within 2 hops',
		);
		expect(graphStanding({ read: 'entry', in: ['ctx-1'], k: 130 }, nameOf)).toBe(
			'entered at ctx-1 within a draw of 130',
		);
	});
});
