// territory.test.ts
import { describe, it, expect } from 'vitest';
import { isEmptyTerritory } from './territory';
import type { Territory } from '$lib/types/generated/graph_territory';

const t = (over: Partial<Territory>): Territory => ({
	id: 'x',
	kind: 'context',
	label: 'X',
	member_count: 3,
	salience: null,
	anchor_id: 'a',
	...over
});

describe('isEmptyTerritory', () => {
	it('true when no members', () => expect(isEmptyTerritory(t({ member_count: 0 }))).toBe(true));
	it('false when populated', () => expect(isEmptyTerritory(t({ member_count: 1 }))).toBe(false));
});
