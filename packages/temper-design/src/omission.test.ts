import { describe, expect, it } from 'vitest';
import {
	type GraphOmissions,
	graphSentence,
	graphStanding,
	omissionSentence,
	uncountedSentence,
} from './omission.js';

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

	it('says a page past the end has every row before it, not its offset', () => {
		expect(omissionSentence(12, 0, 'tasks', { offset: 50, size: 50, more: false })).toBe(
			'None of 12 tasks; 12 before this page.',
		);
	});

	it('states what the count leaves after a page even when the read says nothing follows', () => {
		expect(omissionSentence(50, 48, 'resources', { offset: 0, size: 50, more: false })).toBe(
			'1–48 of 50 resources; 2 after it.',
		);
	});

	it('never says "0 after it" when the read says more follow a stale count', () => {
		expect(omissionSentence(50, 50, 'resources', { offset: 0, size: 50, more: true })).toBe(
			'1–50 of 50 resources; more after it.',
		);
	});

	it('calls a single whole page all of them', () => {
		expect(omissionSentence(7, 7, 'tasks', { offset: 0, size: 50, more: false })).toBe(
			'All 7 tasks.',
		);
	});
});

describe('the omission sentence of a read that counts nothing', () => {
	it('says a read that does not report whether more exist does not, however full the page', () => {
		expect(uncountedSentence(10, 'exact matches', null)).toBe(
			'10 exact matches; more not reported.',
		);
		expect(uncountedSentence(3, 'exact matches', null)).toBe('3 exact matches; more not reported.');
	});

	it('says an unpaged read with more not shown, and calls one without all of them', () => {
		expect(uncountedSentence(10, 'exact matches', { more: true })).toBe(
			'10 exact matches; more not shown.',
		);
		expect(uncountedSentence(10, 'exact matches', { more: false })).toBe('All 10 exact matches.');
	});

	it('calls a first page with nothing after it all of them', () => {
		expect(uncountedSentence(4, 'artifacts', { first: true, more: false })).toBe(
			'All 4 artifacts.',
		);
	});

	it('says which side more lie on, in the cursor’s words and with no count', () => {
		expect(uncountedSentence(50, 'artifacts', { first: true, more: true })).toBe(
			'50 artifacts; more after this page.',
		);
		expect(uncountedSentence(50, 'artifacts', { first: false, more: true })).toBe(
			'50 artifacts; more before this page, more after it.',
		);
		expect(uncountedSentence(7, 'artifacts', { first: false, more: false })).toBe(
			'7 artifacts; more before this page.',
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
