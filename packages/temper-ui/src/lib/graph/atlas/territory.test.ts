// territory.test.ts
import { describe, expect, it } from 'vitest';
import type { Territory } from '$lib/types/generated/graph_territory';
import { isEmptyTerritory } from './territory';

const t = (over: Partial<Territory>): Territory => ({
	id: 'x',
	kind: 'context',
	label: 'X',
	member_count: 3,
	salience: null,
	coherence: null,
	anchor_id: 'a',
	...over,
});

describe('isEmptyTerritory', () => {
	it('true when no members', () => expect(isEmptyTerritory(t({ member_count: 0 }))).toBe(true));
	it('false when populated', () => expect(isEmptyTerritory(t({ member_count: 1 }))).toBe(false));
});
