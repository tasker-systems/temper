import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
	activeFilterCount,
	buildFilterUrl,
	docTypeChips,
	kindScopedClears,
	parseFilters,
	revealedKind,
	STAGES,
	STATUSES,
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

	// Empty means absent, uniformly. `?stage=` used to parse as `''`, which `activeFilterCount`
	// counted as zero while the door still applied it — so the grid said "No resources found."
	// with no Clear-filters link, for a filter the UI did not believe existed.
	it('reads an empty scalar param as absent', () => {
		expect(parseFilters(at('?stage=')).stage).toBeNull();
		expect(parseFilters(at('?status=')).status).toBeNull();
		expect(parseFilters(at('?context_ref=')).contextRef).toBeNull();
		expect(parseFilters(at('?q=')).q).toBeNull();
	});

	it('reads a whitespace-only scalar param as absent', () => {
		expect(parseFilters(at('?stage=%20%20')).stage).toBeNull();
		expect(parseFilters(at('?q=%20')).q).toBeNull();
	});

	it('applies the same rule to the CSV filters', () => {
		expect(parseFilters(at('?doc_type_name=&tags=')).docTypes).toEqual([]);
		expect(parseFilters(at('?doc_type_name=,,&tags=%20,%20')).tags).toEqual([]);
	});

	it('trims a scalar that does carry a value', () => {
		expect(parseFilters(at('?q=%20atlas%20')).q).toBe('atlas');
	});

	// The counter and the door must agree about what is narrowing the set, or the empty state
	// says "No resources found" where it should offer a way out.
	it('makes an empty filter param invisible to the active count, as the door sees it', () => {
		expect(activeFilterCount(parseFilters(at('?stage=')))).toBe(0);
	});
});

// FilterBar's two selects offer these; the schemas define them. A hand-copied enum with no
// guard is how a select comes to offer a value the schema no longer declares — `KIND_KEYS`
// got this guard in the same branch and these did not.
describe('STAGES/STATUSES drift guard', () => {
	const schemaEnum = (kind: string, key: string): string[] => {
		const path = new URL(
			`../../../../crates/temper-workflow/schemas/${kind}.schema.json`,
			import.meta.url,
		);
		const props = JSON.parse(readFileSync(path, 'utf8')).properties ?? {};
		return (props[key]?.enum ?? []).filter((v: unknown) => typeof v === 'string');
	};

	it('offers only stages task.schema.json declares', () => {
		const declared = schemaEnum('task', 'temper-stage');
		expect(declared.length).toBeGreaterThan(0);
		for (const stage of STAGES) {
			expect(declared, `task.schema.json must declare stage ${stage}`).toContain(stage);
		}
	});

	it('offers only statuses goal.schema.json declares', () => {
		const declared = schemaEnum('goal', 'temper-status');
		expect(declared.length).toBeGreaterThan(0);
		for (const status of STATUSES) {
			expect(declared, `goal.schema.json must declare status ${status}`).toContain(status);
		}
	});

	// Subset alone would pass on an empty list; this side catches a schema that GAINED a value.
	it('offers every stage and status the schemas declare', () => {
		expect([...STAGES].sort()).toEqual(schemaEnum('task', 'temper-stage').sort());
		expect([...STATUSES].sort()).toEqual(schemaEnum('goal', 'temper-status').sort());
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

	// A selection that stops revealing `task` also takes away the Stage select — the only
	// control that can clear `stage`. Clearing it in the same mutation is what stops the
	// filter being applied, invisible and unclearable.
	it('clears stage when the selection stops revealing task', () => {
		expect(toggleDocType(at('?doc_type_name=task&stage=done'), 'goal')).not.toContain('stage');
	});

	it('keeps stage when the selection still reveals task', () => {
		expect(toggleDocType(at('?doc_type_name=task,goal&stage=done'), 'goal')).toContain(
			'stage=done',
		);
	});

	it('keeps stage when the selection empties, because the histogram still reveals it', () => {
		expect(toggleDocType(at('?doc_type_name=task&stage=done'), 'task')).toContain('stage=done');
	});
});

describe('kindScopedClears', () => {
	it('clears nothing on an empty selection', () => {
		expect(kindScopedClears([])).toEqual({});
	});

	it('clears the filters the revealed kind does not own', () => {
		expect(kindScopedClears(['task'])).toEqual({ status: null });
		expect(kindScopedClears(['goal'])).toEqual({ stage: null });
	});

	it('clears both when two kinds are selected and nothing is revealed', () => {
		expect(kindScopedClears(['task', 'goal'])).toEqual({ stage: null, status: null });
	});

	it('clears both for a kind that owns neither', () => {
		expect(kindScopedClears(['research'])).toEqual({ stage: null, status: null });
	});
});

describe('docTypeChips', () => {
	it('orders by count descending', () => {
		expect(docTypeChips({ task: 3, goal: 7 }, []).map((c) => c.name)).toEqual(['goal', 'task']);
	});

	it('marks the selected chips', () => {
		expect(docTypeChips({ task: 3, goal: 7 }, ['task'])).toContainEqual({
			name: 'task',
			count: 3,
			active: true,
		});
	});

	// The door emits no zero-count keys, so a selected kind the current filters admit nothing
	// of would render no chip at all — and the chip is the only way to deselect it.
	it('renders a selected kind the histogram omits, at zero', () => {
		expect(docTypeChips({ goal: 7 }, ['task'])).toContainEqual({
			name: 'task',
			count: 0,
			active: true,
		});
	});

	it('renders a selected kind even when the histogram is empty', () => {
		expect(docTypeChips({}, ['task']).map((c) => c.name)).toEqual(['task']);
	});

	it('renders nothing when there is neither a histogram nor a selection', () => {
		expect(docTypeChips(null, [])).toEqual([]);
	});

	it('does not duplicate a selected kind the histogram already counts', () => {
		expect(docTypeChips({ task: 3 }, ['task'])).toHaveLength(1);
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
