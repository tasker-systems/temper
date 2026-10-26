import { WAY_IN_BOUND, WAYS_IN } from '@tasker-systems/temper-design/ways-in';
import { describe, expect, it } from 'vitest';
import type { ContextActivity, ContextRowWithCounts } from '$lib/types';
import { makeRow } from '../test/fixtures';
import {
	ARRIVAL,
	openingOf,
	PLACES_BOUND,
	placesInUse,
	recentPath,
	resumeOf,
	resumePath,
	WAY_IN_MAX,
	wayInMoreHref,
	wayInShown,
} from './first-view';

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

describe('the places a reader works in', () => {
	const place = (n: number, updated: string): ContextRowWithCounts =>
		({
			id: `0000000${n}-0000-7000-8000-000000000000`,
			name: `place ${n}`,
			slug: `place-${n}`,
			owner_ref: '@me',
			updated,
		}) as unknown as ContextRowWithCounts;
	const changed = (n: number, at: string): ContextActivity =>
		({
			context_id: `0000000${n}-0000-7000-8000-000000000000`,
			last_changed_at: at,
		}) as ContextActivity;

	it("orders by the reader's change time, not by the place's own `updated` or its name", () => {
		// Place 1 has the newest `updated` and sorts first by name; the reader changed place 3 last.
		// Which changes count as the reader's is the activity read's to decide, not this function's.
		const contexts = [
			place(1, '2026-10-25T12:00:00Z'),
			place(2, '2026-10-01T00:00:00Z'),
			place(3, '2026-10-02T00:00:00Z'),
		];
		const activity = [
			changed(1, '2026-10-20T00:00:00Z'),
			changed(3, '2026-10-24T00:00:00Z'),
			changed(2, '2026-10-22T00:00:00Z'),
		];
		const { rows, total } = placesInUse(activity, contexts);
		expect(rows.map((r) => r.context.name)).toEqual(['place 3', 'place 2', 'place 1']);
		expect(rows[0].lastChangedAt).toBe('2026-10-24T00:00:00Z');
		expect(total).toBe(3);
	});

	it('leaves out places the reader never changed anything in', () => {
		const { rows } = placesInUse(
			[changed(2, '2026-10-22T00:00:00Z')],
			[place(1, ''), place(2, '')],
		);
		expect(rows.map((r) => r.context.name)).toEqual(['place 2']);
	});

	it('drops a change in a place it cannot name, rather than drawing an unnamed row', () => {
		const { rows, total } = placesInUse([changed(9, '2026-10-22T00:00:00Z')], [place(1, '')]);
		expect(rows).toEqual([]);
		expect(total).toBe(0);
	});

	it('shows at most the bound, and counts every place', () => {
		const n = PLACES_BOUND + 2;
		const ids = Array.from({ length: n }, (_, i) => i);
		const { rows, total } = placesInUse(
			ids.map((i) => changed(i, `2026-10-${10 + i}T00:00:00Z`)),
			ids.map((i) => place(i, '')),
		);
		expect(rows).toHaveLength(PLACES_BOUND);
		expect(total).toBe(n);
	});

	it('fails, rather than claiming none, when the places were never read', () => {
		expect(() => placesInUse([changed(1, '2026-10-22T00:00:00Z')], null)).toThrow();
	});
});

describe('resume', () => {
	it("reads the reader's own changes, newest change of theirs first, one row", () => {
		const params = new URL(resumePath(), 'https://x').searchParams;
		expect(Object.fromEntries(params)).toEqual({
			changed_by: '@me',
			sort: 'changed_by_me',
			order: 'desc',
			limit: '1',
		});
	});

	it("carries the reader's own change time, never the row's `updated`", () => {
		const row = makeRow({
			updated: '2026-10-26T12:00:00Z',
			changed_by_me_at: '2026-10-20T08:00:00Z',
		});
		expect(resumeOf([row])).toEqual({ resource: row, changedAt: '2026-10-20T08:00:00Z' });
	});

	it('offers nothing when the reader has changed nothing', () => {
		expect(resumeOf([])).toBeNull();
	});

	it('fails, rather than falling back to `updated`, when the row has no change time', () => {
		expect(() => resumeOf([makeRow({ updated: '2026-10-26T12:00:00Z' })])).toThrow();
		expect(() => resumeOf([makeRow({ changed_by_me_at: null })])).toThrow();
	});
});

describe('the opening resume shows', () => {
	it('is the first prose, past the headings the body starts with, up to the next heading', () => {
		expect(
			openingOf('# A task\n\n## Problem\n\nThe return.\n\nIt says when.\n\n## Done\nx'),
		).toEqual({
			markdown: 'The return.\n\nIt says when.',
			truncated: true,
		});
	});

	it('is quoted whole, and says nothing continues, when the body is only that', () => {
		expect(openingOf('Just this.\n')).toEqual({ markdown: 'Just this.', truncated: false });
	});

	it('is nothing for a body with no prose, never a stand-in', () => {
		expect(openingOf('')).toBeNull();
		expect(openingOf('# Title\n\n## Empty\n')).toBeNull();
	});

	it('keeps whole blocks while they fit, so the markdown stays well formed', () => {
		const block = 'word '.repeat(30).trim();
		const got = openingOf(`${block}\n\n**${block}**\n\n${block}`, 400);
		expect(got).toEqual({ markdown: `${block}\n\n**${block}**`, truncated: true });
	});

	it('cuts only a first block longer than the bound, at a word', () => {
		const got = openingOf('word '.repeat(200), 100);
		expect(got?.truncated).toBe(true);
		expect(got?.markdown.length).toBeLessThanOrEqual(101);
		expect(got?.markdown).toMatch(/word…$/);
	});

	it('reads a heading inside a fence as code, not as the end of the opening', () => {
		expect(openingOf('Run:\n```\n# not a heading\n\nstill code\n```\n## Next\nx')).toEqual({
			markdown: 'Run:\n```\n# not a heading\n\nstill code\n```',
			truncated: true,
		});
	});
});
