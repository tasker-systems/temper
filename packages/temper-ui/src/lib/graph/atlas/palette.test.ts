// palette.test.ts
import { describe, expect, it } from 'vitest';
import {
	AUTHORED_DOC_TYPES,
	DOC_TYPE_HUES,
	FALLBACK_HUE,
	docTypeHue,
	isAuthored,
	nodeMark,
	paletteStyleVars,
	salienceOpacity
} from './palette';

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

describe('paletteStyleVars', () => {
	it('emits a CSS custom-property string for every doc-type', () => {
		const s = paletteStyleVars();
		expect(s).toContain('--dt-concept:#e8942e');
		expect(s).toContain('--dt-goal:#3a8ae8');
	});
});
