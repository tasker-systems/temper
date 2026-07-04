// legend.test.ts
import { describe, expect, it } from 'vitest';
import { legendModel } from './legend';
import { DOC_TYPE_HUES } from './palette';

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
	it('describes home + edge encodings', () => {
		const m = legendModel();
		expect(m.home).toHaveLength(2); // fill=cogmap, outline=context
		expect(m.edges.length).toBeGreaterThan(0);
	});
});
