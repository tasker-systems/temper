// palette.test.ts
import { describe, expect, it } from 'vitest';
import { AUTHORED_DOC_TYPES, CANVAS_BG, docTypeTint, isAuthored } from './palette';

// `nodeMark`, `salienceOpacity`, `isDocTypeDimmed` and `TERRITORY_TINTS` were deleted in
// Beat D with the tier model, and their tests with them — see palette.ts.

describe('docTypeTint', () => {
	it("paints a doc type in the slot core's vocabulary binds it to", () => {
		expect(docTypeTint('task')).toBe('cat-2');
		expect(docTypeTint('question')).toBe('cat-7');
		expect(docTypeTint('goal')).toBe('cat-5');
	});
	it('paints a doc type bound nowhere, or none, in the neutral', () => {
		expect(docTypeTint('nonsense')).toBe('cat-neutral');
		expect(docTypeTint(null)).toBe('cat-neutral');
	});
	it('binds every doc type the graph knows', () => {
		const all = [
			...AUTHORED_DOC_TYPES,
			'research',
			'task',
			'session',
			'goal',
			'decision',
			'memory',
		];
		for (const t of all) expect(docTypeTint(t)).not.toBe('cat-neutral');
	});
});

describe('isAuthored', () => {
	it('classifies authored vs workflow types', () => {
		expect(isAuthored('concept')).toBe(true);
		expect(isAuthored('goal')).toBe(false);
		expect(isAuthored(null)).toBe(false);
	});
	it('keeps the two families disjoint and covering', () => {
		const workflow = ['research', 'task', 'session', 'goal', 'decision', 'memory'];
		for (const t of AUTHORED_DOC_TYPES) expect(workflow).not.toContain(t);
		expect(AUTHORED_DOC_TYPES.size + workflow.length).toBe(14);
	});
});

describe('chrome', () => {
	// The `TERRITORY_TINTS` half of this block went with the tints in Beat D; the canvas
	// background outlived them and keeps its lock.
	it('paints the canvas on the raised surface role', () => {
		expect(CANVAS_BG).toBe('var(--tp-surface-raised)');
	});
});
