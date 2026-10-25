import { describe, expect, it } from 'vitest';
import { WAYS_IN } from './ways-in.js';

describe('the standing ways in', () => {
	// The set was ruled for both surfaces; a way in added, dropped or renamed here changes what
	// both offer, so it is a ruling to revisit, not an edit to slip through.
	it('are the ruled set, under the names both surfaces show', () => {
		expect(WAYS_IN.map((w) => [w.id, w.label, w.filter])).toEqual([
			['goals', 'goals', { docType: 'goal', owner: '@me', status: 'active' }],
			['tasks', 'in progress', { docType: 'task', owner: '@me', stage: 'in-progress' }],
			['sessions', 'sessions', { docType: 'session', owner: '@me' }],
		]);
	});

	// Ownership is the nearest "yours" a read can answer today, and it is narrower than the work a
	// reader does; the words say whose the list is, so it never reads as more than it holds.
	it("are the reader's own, and say so", () => {
		for (const way of WAYS_IN) {
			expect(way.filter.owner).toBe('@me');
			expect(way.scope).toMatch(/ of yours$/);
		}
	});

	it('say what each holds, and no two the same', () => {
		const scopes = WAYS_IN.map((w) => w.scope);
		expect(scopes.every((s) => s.trim().length > 0)).toBe(true);
		expect(new Set(scopes).size).toBe(scopes.length);
	});
});
