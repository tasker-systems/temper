// palette.test.ts
import { describe, expect, it } from 'vitest';
import type { AtlasEdge } from '$lib/types/generated/graph_atlas';
import {
	AUTHORED_DOC_TYPES,
	CANVAS_BG,
	docTypeTint,
	EDGE_COLORS,
	edgeStyle,
	isAuthored,
} from './palette';

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
	it('paints the edges on roles: structural, contradicts and derived stay distinct', () => {
		expect(EDGE_COLORS).toEqual({
			structural: 'var(--tp-text-muted)',
			contradicts: 'var(--tp-danger)',
			derived: 'var(--tp-text-subtle)',
		});
	});
});

const edge = (o: Partial<AtlasEdge>): AtlasEdge => ({
	id: 'e',
	source: 's',
	target: 't',
	edge_kind: 'contains',
	polarity: 'forward',
	label: null,
	weight: 1,
	...o,
});

describe('edgeStyle', () => {
	it('maps edge_kind to line style', () => {
		expect(edgeStyle(edge({ edge_kind: 'contains' })).dash).toBeNull();
		expect(edgeStyle(edge({ edge_kind: 'leads_to' })).dash).toBe('7 4');
		expect(edgeStyle(edge({ edge_kind: 'express' })).dash).toBe('1 4');
		expect(edgeStyle(edge({ edge_kind: 'near' })).dash).toBe('4 4');
	});
	it('derived_from label → provenance color + dashed regardless of kind', () => {
		const s = edgeStyle(edge({ edge_kind: 'contains', label: 'derived_from' }));
		expect(s.color).toBe(EDGE_COLORS.derived);
		expect(s.dash).toBe('7 4');
	});
	it('contradicts label → warning red', () => {
		expect(edgeStyle(edge({ label: 'contradicts' })).color).toBe(EDGE_COLORS.contradicts);
	});
	/**
	 * Contradicts must differ from structural on more than hue: at their luminances (about 1.2:1 in
	 * dark, 1.4:1 in paper) a colour-blind reader cannot tell them apart. Dash already says the
	 * edge's kind and width its weight, so contradicts carries a shape of its own — a cross at the
	 * midpoint — and keeps both.
	 */
	it('contradicts carries a cross, and keeps the kind dash and the weight width', () => {
		const contradicts = edgeStyle(edge({ label: 'contradicts', edge_kind: 'leads_to', weight: 3 }));
		const structural = edgeStyle(edge({ edge_kind: 'leads_to', weight: 3 }));
		expect(contradicts.cross).toBe(true);
		expect(structural.cross).toBe(false);
		expect(contradicts.dash).toBe(structural.dash);
		expect(contradicts.width).toBe(structural.width);
	});
	it('default color is structural gray', () => {
		expect(edgeStyle(edge({})).color).toBe(EDGE_COLORS.structural);
	});
	it('weight → thickness clamped to [1,5]', () => {
		expect(edgeStyle(edge({ weight: 0.2 })).width).toBe(1);
		expect(edgeStyle(edge({ weight: 3 })).width).toBe(3);
		expect(edgeStyle(edge({ weight: 99 })).width).toBe(5);
	});
	it('polarity → arrowhead; near is symmetric (no marker)', () => {
		expect(edgeStyle(edge({ polarity: 'forward' }))).toMatchObject({
			markerEnd: true,
			markerStart: false,
		});
		expect(edgeStyle(edge({ polarity: 'inverse' }))).toMatchObject({
			markerEnd: false,
			markerStart: true,
		});
		const n = edgeStyle(edge({ edge_kind: 'near', polarity: 'forward' }));
		expect(n.markerStart).toBe(false);
		expect(n.markerEnd).toBe(false);
	});
});
