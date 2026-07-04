// legend.test.ts
import { describe, expect, it } from 'vitest';
import { legendModel } from './legend';
import { DOC_TYPE_HUES, EDGE_COLORS, EDGE_KINDS } from './palette';

describe('legendModel', () => {
	it('lists every doc-type hue from the palette (no drift)', () => {
		const m = legendModel();
		const swatchTypes = m.docTypes.map((d) => d.docType).sort();
		expect(swatchTypes).toEqual(Object.keys(DOC_TYPE_HUES).sort());
	});
	it('groups authored vs workflow', () => {
		const m = legendModel();
		expect(m.docTypes.some((d) => d.authored)).toBe(true);
		expect(m.docTypes.some((d) => !d.authored)).toBe(true);
	});
	it('describes home + edge color encodings', () => {
		const m = legendModel();
		expect(m.home).toHaveLength(2); // fill=cogmap, outline=context
		expect(m.edgeColors.length).toBeGreaterThan(0);
	});
});

describe('legendModel edge grammar (kind/color/polarity/weight)', () => {
	it('has a dash entry for every EdgeKind — no drift against palette.ts (the ScopeBar filter axis)', () => {
		const m = legendModel();
		const kinds = m.edgeKinds.map((e) => e.kind).sort();
		expect(kinds).toEqual([...EDGE_KINDS].sort());
	});

	it('sources each kind dash from edgeStyle: contains is solid, the rest are dashed', () => {
		const m = legendModel();
		const contains = m.edgeKinds.find((e) => e.kind === 'contains');
		expect(contains?.dash).toBeNull();
		const dashedKinds = m.edgeKinds.filter((e) => e.kind !== 'contains');
		expect(dashedKinds.length).toBeGreaterThan(0);
		for (const k of dashedKinds) expect(k.dash).not.toBeNull();
	});

	it('keeps edge color labels sourced from EDGE_COLORS (structural/contradicts/derived)', () => {
		const m = legendModel();
		expect(m.edgeColors.map((e) => e.label).sort()).toEqual(Object.keys(EDGE_COLORS).sort());
	});

	it('derives polarity markers from edgeStyle: forward=arrow-end, inverse=arrow-start, near=symmetric/no marker', () => {
		const m = legendModel();
		expect(m.polarity).toHaveLength(3);
		const forward = m.polarity.find((p) => p.label.toLowerCase().includes('forward'));
		const inverse = m.polarity.find((p) => p.label.toLowerCase().includes('inverse'));
		const near = m.polarity.find((p) => p.label.toLowerCase().includes('near'));
		expect(forward?.marker).toBe('end');
		expect(inverse?.marker).toBe('start');
		expect(near?.marker).toBe('none');
	});

	it('shows weight samples with strictly increasing stroke width, from edgeStyle', () => {
		const m = legendModel();
		expect(m.weight.length).toBeGreaterThan(1);
		const widths = m.weight.map((w) => w.width);
		for (let i = 1; i < widths.length; i++) {
			expect(widths[i]).toBeGreaterThan(widths[i - 1]);
		}
	});
});
