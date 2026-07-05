// trail.test.ts
import { describe, expect, it } from 'vitest';
import { trailModel } from './trail';
import type { EventTrail } from '$lib/types/generated/element_trail';

const trail = (events: EventTrail['events']): EventTrail => ({ element_kind: 'node', element_id: 'n1', events });

describe('trailModel', () => {
	it('maps events newest-first and humanizes kind', () => {
		const t = trail([
			{ event_id: 'a', kind: 'relationship.asserted', actor_entity_id: 'u1', occurred_at: '2026-01-01T00:00:00Z', confidence: null },
			{ event_id: 'b', kind: 'relationship.reweighted', actor_entity_id: 'u1', occurred_at: '2026-02-01T00:00:00Z', confidence: 'high' }
		]);
		const rows = trailModel(t);
		expect(rows[0]).toMatchObject({ kind: 'Reweighted', occurredAt: '2026-02-01T00:00:00Z', confidence: 'high' });
		expect(rows[1]).toMatchObject({ kind: 'Asserted', confidence: null });
	});
	it('normalizes missing confidence to null', () => {
		const t = trail([{ event_id: 'a', kind: 'block.created', actor_entity_id: 'u', occurred_at: '2026-01-01T00:00:00Z', confidence: undefined as unknown as null }]);
		expect(trailModel(t)[0].confidence).toBeNull();
	});
});
