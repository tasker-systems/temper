import { fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ContextRowWithCounts } from '$lib/types';
import type { VaultFilters } from '$lib/vault-filters';
import { goto, gotoTarget, resetAppContext, setPage } from '../../../test/app-context';
import FilterBar from './FilterBar.svelte';

vi.mock('$app/stores', () => import('../../../test/app-context'));
vi.mock('$app/navigation', () => import('../../../test/app-context'));

beforeEach(resetAppContext);
afterEach(() => {
	vi.useRealTimers();
});

const NO_FILTERS: VaultFilters = {
	docTypes: [],
	stage: null,
	status: null,
	contextRef: null,
	q: null,
	tags: [],
};

/** `contexts` defaults to `[]` — a read that answered with nothing, not a read that failed. */
function mount(
	revealed: string | null,
	filters: Partial<VaultFilters> = {},
	contexts: ContextRowWithCounts[] | null = [],
) {
	return render(FilterBar, {
		props: { filters: { ...NO_FILTERS, ...filters }, revealed, fixedContext: false, contexts },
	});
}

/**
 * `buildFilterUrl` is pinned by `vault-filters.test.ts` and the draft reducer by
 * `vault-draft.test.ts`. Neither can see whether a control is CONNECTED to them: a handler
 * that computes the right URL and never calls `goto`, or a debounce that never fires, is
 * green in both. The two paths differ in kind and are asserted separately — the text field
 * navigates through a 300ms `setTimeout`, the selects navigate on the change event itself.
 */
describe('FilterBar — the control that mutates the URL', () => {
	it('holds the navigation for the whole 300ms, then fires it once', async () => {
		vi.useFakeTimers();
		setPage('/vault/all');

		const { getByLabelText } = mount(null);
		await fireEvent.input(getByLabelText('title contains'), { target: { value: 'ledger' } });

		// Three assertions, because only the middle one has a lower bound to fail against.
		// Asserting "not called" on the keystroke alone is vacuous under fake timers — the
		// clock has not advanced, so a `setTimeout(fn, 0)` has not fired either, and a 0ms
		// debounce IS the six-navigations-per-word defect this test is named for.
		expect(goto).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(299);
		expect(goto).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);
		expect(goto).toHaveBeenCalledTimes(1);
	});

	it('navigates with the typed value once the 300ms debounce elapses', async () => {
		vi.useFakeTimers();
		setPage('/vault/all?doc_type_name=task&offset=40');

		const { getByLabelText } = mount('task');
		await fireEvent.input(getByLabelText('title contains'), { target: { value: 'ledger' } });
		// `...Async` runs the debounce callback AND drains the microtask queue, so the
		// `.finally()` the handler chains onto the navigation has settled before we assert.
		// Plain `advanceTimersByTime` would assert against a component still mid-flight.
		await vi.advanceTimersByTimeAsync(300);

		expect(goto).toHaveBeenCalledTimes(1);
		const target = gotoTarget();
		expect(target.pathname).toBe('/vault/all');
		expect(target.searchParams.get('q')).toBe('ledger');
		// The rest of the URL is what `buildFilterUrl` made of it — carried through, not
		// rebuilt from scratch, and paged back to the first page.
		expect(target.searchParams.get('doc_type_name')).toBe('task');
		expect(target.searchParams.get('offset')).toBeNull();
	});

	it('navigates on the stage select changing, with no timer to advance', async () => {
		setPage('/vault/all?doc_type_name=task&offset=40');

		const { getByLabelText } = mount('task');
		await fireEvent.change(getByLabelText('stage'), { target: { value: 'done' } });

		// Real timers, nothing advanced: a select is a committed choice rather than a stream
		// of keystrokes, so it is wired straight to `navigate`. Sharing the text field's
		// debounce would be a different defect from being wired to nothing at all.
		expect(goto).toHaveBeenCalledTimes(1);
		expect(gotoTarget().searchParams.get('stage')).toBe('done');
	});
});

/**
 * A filter that does not apply to the revealed kind is not offered greyed-out — it is not
 * there. A disabled select still asserts "this filter exists for goals, just not now", and
 * `stage` does not exist for goals at all. `queryByLabelText` returns the element whether or
 * not it is disabled, so these assertions fail on a control rendered-and-disabled.
 */
describe('FilterBar — a kind-scoped filter the revealed kind does not have', () => {
	it('offers stage, and no status, when task is the revealed kind', () => {
		const { getByLabelText, queryByLabelText } = mount('task');

		expect(getByLabelText('stage')).toBeDefined();
		expect(queryByLabelText('status')).toBeNull();
	});

	it('offers status, and removes stage from the DOM, when goal is the revealed kind', () => {
		// `stage: 'done'` is the stranded-filter case: the door is still applying it and the
		// control that could clear it is gone. `kindScopedClears` is what stops that URL
		// arising; what is asserted here is only that the control really is gone.
		const { getByLabelText, queryByLabelText } = mount('goal', { stage: 'done' });

		expect(getByLabelText('status')).toBeDefined();
		expect(queryByLabelText('stage')).toBeNull();
	});

	it('offers neither when no single kind is revealed', () => {
		const { queryByLabelText } = mount(null);

		expect(queryByLabelText('stage')).toBeNull();
		expect(queryByLabelText('status')).toBeNull();
	});
});

/**
 * The third rendered state, and the one with no pure module behind it at all: `contexts` is
 * `ContextRowWithCounts[] | null`, and only this component can tell the two apart on screen.
 * An empty select says "there is nothing to filter by" — a claim about the vault that a fetch
 * which never answered cannot support — so the failed read renders inert and marked instead.
 */
describe('FilterBar — a context read that never answered', () => {
	it('offers an inert marked select, not an empty one, when the read failed', () => {
		const { getByLabelText } = mount(null, {}, null);
		const select = getByLabelText('context') as HTMLSelectElement;

		expect(select.disabled).toBe(true);
		expect(select.textContent).toContain('contexts unavailable');
		// The distinguishing assertion: an empty read would offer "All contexts" and nothing
		// else, which is a select the reader can operate and believe.
		expect(select.textContent).not.toContain('All contexts');
	});

	it('offers an operable select with no options to choose when the read answered empty', () => {
		const { getByLabelText } = mount(null, {}, []);
		const select = getByLabelText('context') as HTMLSelectElement;

		expect(select.disabled).toBe(false);
		expect(select.textContent).toContain('All contexts');
	});
});
