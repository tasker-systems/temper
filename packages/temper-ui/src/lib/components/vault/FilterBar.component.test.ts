import { fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { VaultFilters } from '$lib/vault-filters';
import { goto, resetAppContext, setPage } from '../../../test/app-context';
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

/** `contexts: []` on purpose — the `contexts === null` arm is a different behaviour. */
function mount(revealed: string | null, filters: Partial<VaultFilters> = {}) {
	return render(FilterBar, {
		props: {
			filters: { ...NO_FILTERS, ...filters },
			revealed,
			fixedContext: false,
			contexts: [],
		},
	});
}

function gotoTarget(call: number): URL {
	return new URL(String(goto.mock.calls[call][0]), 'http://localhost');
}

/**
 * `buildFilterUrl` is pinned by `vault-filters.test.ts` and the draft reducer by
 * `vault-draft.test.ts`. Neither can see whether a control is CONNECTED to them: a handler
 * that computes the right URL and never calls `goto`, or a debounce that never fires, is
 * green in both. The two paths differ in kind and are asserted separately — the text field
 * navigates through a 300ms `setTimeout`, the selects navigate on the change event itself.
 */
describe('FilterBar — the control that mutates the URL', () => {
	it('does not navigate on the keystroke itself', async () => {
		vi.useFakeTimers();
		setPage('/vault/all');

		const { getByLabelText } = mount(null);
		await fireEvent.input(getByLabelText('title contains'), { target: { value: 'ledger' } });

		// The debounce is the whole reason a six-character word is one navigation, not six.
		expect(goto).not.toHaveBeenCalled();
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
		const target = gotoTarget(0);
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
		expect(gotoTarget(0).searchParams.get('stage')).toBe('done');
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
