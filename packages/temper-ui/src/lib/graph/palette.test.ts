// palette.test.ts
import { describe, expect, it } from 'vitest';
import type { AtlasEdge } from '$lib/types/generated/graph_atlas';
import {
	AUTHORED_DOC_TYPES,
	CANVAS_BG,
	DOC_TYPE_HUES,
	docTypeHue,
	EDGE_COLORS,
	edgeStyle,
	FALLBACK_HUE,
	isAuthored,
	paletteStyleVars,
} from './palette';

// `nodeMark`, `salienceOpacity`, `isDocTypeDimmed` and `TERRITORY_TINTS` were deleted in
// Beat D with the tier model, and their tests with them — see palette.ts.

describe('DOC_TYPE_HUES', () => {
	it('defines all 14 doc-types with the locked Vivid Cartographer hexes', () => {
		expect(DOC_TYPE_HUES.concept).toBe('#e8942e');
		expect(DOC_TYPE_HUES.fact).toBe('#f7c62b');
		expect(DOC_TYPE_HUES.domain).toBe('#d3d84e');
		expect(DOC_TYPE_HUES.goal).toBe('#3a8ae8'); // goal is cool (legacy gold retired)
		expect(Object.keys(DOC_TYPE_HUES)).toHaveLength(14);
	});
});

describe('docTypeHue', () => {
	it('returns the hue for a known type', () => {
		expect(docTypeHue('question')).toBe('#a95cf0');
	});
	it('falls back for unknown or null', () => {
		expect(docTypeHue('nonsense')).toBe(FALLBACK_HUE);
		expect(docTypeHue(null)).toBe(FALLBACK_HUE);
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
	it('locks the canvas background', () => {
		expect(CANVAS_BG).toBe('#1b1e26');
	});
});

describe('paletteStyleVars', () => {
	it('emits a CSS custom-property string for every doc-type', () => {
		const s = paletteStyleVars();
		expect(s).toContain('--dt-concept:#e8942e');
		expect(s).toContain('--dt-goal:#3a8ae8');
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
