import { describe, expect, it } from 'vitest';
import { summarizeEvent } from './eventSummary';

describe('summarizeEvent', () => {
	it('summarizes property_set as key → value', () => {
		expect(summarizeEvent('property_set', { property_key: 'temper-stage', value: 'in-progress' }))
			.toBe('temper-stage → in-progress');
	});
	it('summarizes relationship_asserted with a resolved target title', () => {
		const nodes = new Map([['t1', { title: 'Cutover checklist' }]]);
		expect(
			summarizeEvent(
				'relationship_asserted',
				{ label: 'derived_from', target: { id: 't1' } },
				nodes
			)
		).toBe('derived_from → Cutover checklist');
	});
	it('falls back to the relationship label when the target is unknown', () => {
		expect(summarizeEvent('relationship_asserted', { label: 'part_of', target: { id: 'zzz' } }))
			.toBe('part_of');
	});
	it('returns null for kinds with no useful summary', () => {
		expect(summarizeEvent('resource_created', { title: 'x' })).toBeNull();
	});
	it('never throws on malformed payloads', () => {
		expect(summarizeEvent('property_set', null)).toBeNull();
	});
});
