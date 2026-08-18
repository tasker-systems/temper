import { describe, expect, it } from 'vitest';
import {
	activeFilterCount,
	buildFilterUrl,
	parseFilters,
	revealedKind,
	toggleDocType,
} from './vault-filters';

const at = (search: string) => new URL(`https://x.test/vault/all${search}`);

describe('parseFilters', () => {
	it('reads an empty URL as no filters', () => {
		expect(parseFilters(at(''))).toEqual({
			docTypes: [],
			stage: null,
			status: null,
			contextRef: null,
			q: null,
			tags: [],
		});
	});

	it('splits doc_type_name as CSV', () => {
		expect(parseFilters(at('?doc_type_name=task,goal')).docTypes).toEqual(['task', 'goal']);
	});

	it('trims CSV members and drops empties', () => {
		expect(parseFilters(at('?doc_type_name=task,%20,goal,')).docTypes).toEqual(['task', 'goal']);
	});
});

describe('buildFilterUrl', () => {
	it('resets offset whenever a filter changes', () => {
		const out = buildFilterUrl(at('?offset=100&sort=title'), { stage: 'done' });
		expect(out).not.toContain('offset');
		expect(out).toContain('sort=title');
		expect(out).toContain('stage=done');
	});

	it('deletes a param set to null rather than writing an empty value', () => {
		expect(buildFilterUrl(at('?stage=done'), { stage: null })).not.toContain('stage');
	});

	it('encodes docTypes back to CSV', () => {
		expect(buildFilterUrl(at(''), { docTypes: ['task', 'goal'] })).toContain(
			'doc_type_name=task%2Cgoal',
		);
	});
});

describe('toggleDocType', () => {
	it('adds a kind that is not selected', () => {
		expect(toggleDocType(at('?doc_type_name=task'), 'goal')).toContain('task%2Cgoal');
	});

	it('removes a kind that is selected', () => {
		const out = toggleDocType(at('?doc_type_name=task,goal'), 'task');
		expect(out).toContain('doc_type_name=goal');
		expect(out).not.toContain('task');
	});

	it('drops the param entirely when the last kind is removed', () => {
		expect(toggleDocType(at('?doc_type_name=task'), 'task')).not.toContain('doc_type_name');
	});
});

describe('revealedKind', () => {
	const none = parseFilters(at(''));

	it('is the sole selected kind', () => {
		expect(revealedKind(parseFilters(at('?doc_type_name=task')), { task: 3, goal: 2 })).toBe(
			'task',
		);
	});

	it('is null when two kinds are selected', () => {
		expect(
			revealedKind(parseFilters(at('?doc_type_name=task,goal')), { task: 3, goal: 2 }),
		).toBeNull();
	});

	// The arm that distinguishes the two rules: with no selection the histogram DOES
	// describe the fully filtered set, because excluding an absent predicate changes nothing.
	it('falls back to the histogram when nothing is selected', () => {
		expect(revealedKind(none, { task: 3 })).toBe('task');
	});

	it('is null when the histogram holds more than one kind and nothing is selected', () => {
		expect(revealedKind(none, { task: 3, goal: 2 })).toBeNull();
	});

	it('ignores zero-count histogram entries', () => {
		expect(revealedKind(none, { task: 3, goal: 0 })).toBe('task');
	});

	it('is null on an empty histogram', () => {
		expect(revealedKind(none, {})).toBeNull();
	});
});

describe('activeFilterCount', () => {
	it('is zero on an unfiltered URL', () => {
		expect(activeFilterCount(parseFilters(at('')))).toBe(0);
	});

	it('ignores sort, order and offset, which do not narrow', () => {
		expect(activeFilterCount(parseFilters(at('?sort=title&order=asc&offset=50')))).toBe(0);
	});

	it('counts a multi-value doc-type selection once', () => {
		expect(activeFilterCount(parseFilters(at('?doc_type_name=task,goal')))).toBe(1);
	});

	it('counts each distinct dimension', () => {
		expect(activeFilterCount(parseFilters(at('?doc_type_name=task&stage=done&q=atlas')))).toBe(3);
	});
});
