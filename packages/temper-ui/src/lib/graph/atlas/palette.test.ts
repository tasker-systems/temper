// palette.test.ts
import { describe, expect, it } from 'vitest';
import {
	AUTHORED_DOC_TYPES,
	CANVAS_BG,
	DOC_TYPE_HUES,
	EDGE_COLORS,
	FALLBACK_HUE,
	TEAM_ZONE,
	TERRITORY_TINTS,
	docTypeHue,
	edgeStyle,
	isAuthored,
	isDocTypeDimmed,
	nodeMark,
	paletteStyleVars,
	salienceOpacity
} from './palette';
import type { AtlasEdge } from '$lib/types/generated/graph_atlas';

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

describe('nodeMark', () => {
	it('fills cogmap-homed nodes and outlines context-homed ones', () => {
		expect(nodeMark('concept', 'cogmap')).toEqual({ color: '#e8942e', filled: true });
		expect(nodeMark('research', 'context')).toEqual({ color: '#33b0e2', filled: false });
	});
});

describe('salienceOpacity', () => {
	it('ramps within [0.35, 1] and clamps', () => {
		expect(salienceOpacity(0)).toBeCloseTo(0.35);
		expect(salienceOpacity(1)).toBeCloseTo(1);
		expect(salienceOpacity(2)).toBeCloseTo(1); // clamp high
		expect(salienceOpacity(null)).toBeCloseTo(0.35); // null → floor
	});
});

describe('isDocTypeDimmed', () => {
	it('never dims when the filter set is empty', () => {
		expect(isDocTypeDimmed('concept', [])).toBe(false);
		expect(isDocTypeDimmed(null, [])).toBe(false);
	});
	it('keeps matching doc-types at full opacity', () => {
		expect(isDocTypeDimmed('concept', ['concept', 'task'])).toBe(false);
	});
	it('dims non-matching doc-types', () => {
		expect(isDocTypeDimmed('fact', ['concept', 'task'])).toBe(true);
	});
	it('dims a null doc-type once any filter is active', () => {
		expect(isDocTypeDimmed(null, ['concept'])).toBe(true);
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
	...o
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
		expect(edgeStyle(edge({ polarity: 'forward' }))).toMatchObject({ markerEnd: true, markerStart: false });
		expect(edgeStyle(edge({ polarity: 'inverse' }))).toMatchObject({ markerEnd: false, markerStart: true });
		const n = edgeStyle(edge({ edge_kind: 'near', polarity: 'forward' }));
		expect(n.markerStart).toBe(false);
		expect(n.markerEnd).toBe(false);
	});
});

describe('TERRITORY_TINTS / chrome', () => {
	it('defines all three territory kinds with non-empty hex strings', () => {
		for (const kind of ['region', 'context', 'cogmap'] as const) {
			expect(TERRITORY_TINTS[kind]).toMatch(/^#[0-9a-f]{6}$/i);
		}
	});

	it('locks the canvas background', () => {
		expect(CANVAS_BG).toBe('#1b1e26');
	});

	it('defines team-zone fill, label, and sub colors', () => {
		expect(TEAM_ZONE.fill).toMatch(/^#[0-9a-f]{6}$/i);
		expect(TEAM_ZONE.label).toMatch(/^#[0-9a-f]{6}$/i);
		expect(TEAM_ZONE.sub).toMatch(/^#[0-9a-f]{6}$/i);
	});
});
