import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
	activeFilterCount,
	buildFilterUrl,
	docTypeChips,
	doorParams,
	FILTER_PARAM_KEYS,
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
	// says "No resources found" where it should offer a way out. Counting it as zero is only
	// half of that agreement — the other half is that the loaders never forward it. (Asserting
	// the count alone constrained nothing: `''` was already falsy before the parse was fixed.)
	it('hides an empty filter param from the count AND from the door', () => {
		expect(activeFilterCount(parseFilters(at('?stage=')))).toBe(0);
		expect(doorParams(at('?stage=').searchParams).has('stage')).toBe(false);
	});
});

describe('doorParams', () => {
	const door = (search: string) => doorParams(at(search).searchParams);

	// The finding this closes: the door applied `Some("")` to every row while the UI counted
	// zero active filters, so the grid said "No resources found." with no Clear-filters link.
	it('drops an empty filter param instead of forwarding it', () => {
		expect(door('?stage=').has('stage')).toBe(false);
		expect(door('?q=').has('q')).toBe(false);
		expect([...door('?stage=&status=&context_ref=&q=&doc_type_name=&tags=').keys()]).toEqual([]);
	});

	// The regression the parse-only fix introduced: `?stage=%20` used to count as one filter and
	// render the way out; parsed-as-absent it counted as zero while the door still narrowed on
	// `' '`. Stripping it here is what makes the zero count true.
	it('drops a whitespace-only filter param', () => {
		expect(door('?stage=%20').has('stage')).toBe(false);
		expect(door('?q=%20%20').has('q')).toBe(false);
	});

	it('leaves a filter that carries a value untouched', () => {
		const params = door('?stage=done&q=atlas&doc_type_name=task,goal&tags=ci');
		expect(params.get('stage')).toBe('done');
		expect(params.get('q')).toBe('atlas');
		expect(params.get('doc_type_name')).toBe('task,goal');
		expect(params.get('tags')).toBe('ci');
	});

	// Not filters: `?limit=` is a 400 the page surfaces, not a silently-unfiltered page, and
	// rewriting it here would turn a rejected request into a default-limit one.
	it('leaves the non-filter params alone, empty or not', () => {
		const params = door('?limit=&offset=&sort=&order=&owner=');
		for (const key of ['limit', 'offset', 'sort', 'order', 'owner']) {
			expect(params.get(key), key).toBe('');
		}
	});

	it('keeps the non-empty values of a repeated filter param', () => {
		const params = door('?stage=&stage=done');
		expect(params.getAll('stage')).toEqual(['done']);
	});

	it("does not mutate the caller's params", () => {
		const source = at('?stage=').searchParams;
		doorParams(source);
		expect(source.get('stage')).toBe('');
	});

	// The stripped set and the parsed set are the same set, in both directions — a filter
	// `parseFilters` reads but `doorParams` does not strip is the exact shape of the original
	// finding, and one it strips but the parse ignores would silently drop a live filter.
	it('strips exactly the params parseFilters interprets', () => {
		for (const key of FILTER_PARAM_KEYS) {
			expect(activeFilterCount(parseFilters(at(`?${key}=x`))), key).toBe(1);
			expect(door(`?${key}=x`).get(key), key).toBe('x');
			expect(activeFilterCount(parseFilters(at(`?${key}=%20`))), key).toBe(0);
			expect(door(`?${key}=%20`).has(key), key).toBe(false);
		}
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
