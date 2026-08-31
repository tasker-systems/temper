import { describe, expect, it } from 'vitest';
import { summarizeEvent } from './eventSummary';

describe('summarizeEvent', () => {
	it('summarizes property_set as key → value', () => {
		expect(
			summarizeEvent('property_set', { property_key: 'temper-stage', value: 'in-progress' }),
		).toBe('temper-stage → in-progress');
	});
	it('summarizes relationship_asserted with a resolved target title', () => {
		const nodes = new Map([['t1', { title: 'Cutover checklist' }]]);
		expect(
			summarizeEvent(
				'relationship_asserted',
				{ label: 'derived_from', target: { id: 't1' } },
				nodes,
			),
		).toBe('derived_from → Cutover checklist');
	});
	it('falls back to the relationship label when the target is unknown', () => {
		expect(
			summarizeEvent('relationship_asserted', { label: 'part_of', target: { id: 'zzz' } }),
		).toBe('part_of');
	});
	it('summarizes data_artifact_committed as family · intent · size · hash · supersession', () => {
		expect(
			summarizeEvent('data_artifact_committed', {
				artifact_id: '01a0',
				resource_id: 'r1',
				artifact_kind: 'measurement',
				intent: 'member',
				precedence: 0,
				content_hash: 'a'.repeat(64),
				content_bytes: 1229,
				supersedes: ['old1'],
			}),
		).toBe(`measurement · member · 1.2 KB · sha256:${'a'.repeat(8)}… · supersedes 1`);
	});
	it('omits size, hash and supersession when the payload does not carry them', () => {
		expect(
			summarizeEvent('data_artifact_committed', {
				artifact_kind: 'extraction',
				intent: 'current',
				content_bytes: 0,
				supersedes: [],
			}),
		).toBe('extraction · current · 0 B');
	});
	it('keeps bytes exact under a unit and never invents them', () => {
		expect(
			summarizeEvent('data_artifact_committed', {
				artifact_kind: 'measurement',
				content_bytes: 1536,
			}),
		).toBe('measurement · 1.5 KB');
		expect(
			summarizeEvent('data_artifact_committed', {
				artifact_kind: 'measurement',
				content_bytes: 'many',
			}),
		).toBe('measurement');
	});
	it('returns null for a data_artifact_committed payload without a family', () => {
		expect(summarizeEvent('data_artifact_committed', { content_bytes: 12 })).toBeNull();
	});
	it('returns null for kinds with no useful summary', () => {
		expect(summarizeEvent('resource_created', { title: 'x' })).toBeNull();
	});
	it('never throws on malformed payloads', () => {
		expect(summarizeEvent('property_set', null)).toBeNull();
	});
});
