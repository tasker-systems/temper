import { WAY_IN_BOUND, WAYS_IN } from '@tasker-systems/temper-design/ways-in';
import { describe, expect, it } from 'vitest';
import { ARRIVAL, recentPath, WAY_IN_MAX, wayInMoreHref, wayInShown } from './first-view';

const goals = WAYS_IN[0];

describe('how far a way in has grown', () => {
	it('is the shared bound unless the URL asks for more', () => {
		expect(wayInShown(new URLSearchParams(), goals)).toBe(WAY_IN_BOUND);
		expect(wayInShown(new URLSearchParams({ goals: '16' }), goals)).toBe(16);
	});

	it('never reads below the bound, past the most, or from something unreadable', () => {
		for (const asked of ['2', '-8', '8.5', 'many', '']) {
			expect(wayInShown(new URLSearchParams({ goals: asked }), goals)).toBe(WAY_IN_BOUND);
		}
		expect(wayInShown(new URLSearchParams({ goals: '100000' }), goals)).toBe(WAY_IN_MAX);
	});

	it('grows by one step and keeps every other list where it was', () => {
		const url = new URL(`https://temper.test${ARRIVAL}?tasks=24`);
		const href = wayInMoreHref(url, goals, WAY_IN_BOUND);
		const params = new URL(href, url).searchParams;
		expect(new URL(href, url).pathname).toBe(ARRIVAL);
		expect(params.get('goals')).toBe(String(WAY_IN_BOUND * 2));
		expect(params.get('tasks')).toBe('24');
	});
});

describe('recently updated', () => {
	// It is not scoped to the reader: the web holds no signal for where they work, and the scope
	// words say "resources you can read" rather than claiming one.
	it('reads everything visible, bounded, newest update first, with no owner narrowing', () => {
		expect(Object.fromEntries(new URL(recentPath(), 'https://x').searchParams)).toEqual({
			limit: '5',
			sort: 'updated',
			order: 'desc',
		});
	});
});
