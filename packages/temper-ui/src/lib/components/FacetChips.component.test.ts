import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { goto, resetAppContext, setPage } from '../../test/app-context';
import FacetChips from './FacetChips.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));
vi.mock('$app/navigation', () => import('../../test/app-context'));

beforeEach(resetAppContext);

/**
 * `docTypeChips` — the value half — is already pinned by `vault-filters.test.ts`, including
 * the zero-count case. Nothing here re-asserts it. What is untested until now is the WIRING:
 * that the component renders from `docTypeChips(facets, selected)` rather than from the raw
 * histogram it is handed, and that the chip it puts on screen is one the reader can act on.
 */
describe('FacetChips — a selected kind the histogram cannot see', () => {
	it('renders a chip for a selected kind absent from the histogram', () => {
		setPage('/vault/all?doc_type_name=blueprint');

		const { getByRole } = render(FacetChips, { props: { facets: { task: 7 } } });

		// The door emits no zero-count keys, so `blueprint` is nowhere in `facets`. A component
		// iterating the histogram it was passed renders one chip here and the reader is stuck
		// with a filter they cannot see or clear.
		expect(getByRole('button', { name: /blueprint/ })).toBeDefined();
	});

	it('marks that chip selected and shows the zero it admits', () => {
		setPage('/vault/all?doc_type_name=blueprint');

		const { getByRole } = render(FacetChips, { props: { facets: { task: 7 } } });
		const chip = getByRole('button', { name: /blueprint/ });

		expect(chip.getAttribute('aria-pressed')).toBe('true');
		expect(chip.textContent).toContain('0');
	});

	it('deselects that kind when the chip is clicked — the only control that can', () => {
		setPage('/vault/all?doc_type_name=blueprint');

		const { getByRole } = render(FacetChips, { props: { facets: { task: 7 } } });
		getByRole('button', { name: /blueprint/ }).click();

		expect(goto).toHaveBeenCalledTimes(1);
		const target = new URL(String(goto.mock.calls[0][0]), 'http://localhost');
		expect(target.searchParams.getAll('doc_type_name')).toEqual([]);
	});
});

describe('FacetChips — the histogram it was handed', () => {
	it('renders a chip per counted kind when nothing is selected', () => {
		setPage('/vault/all');

		const { getAllByRole } = render(FacetChips, { props: { facets: { task: 7, goal: 2 } } });

		expect(getAllByRole('button').map((b) => b.textContent?.trim().split(/\s+/)[0])).toEqual([
			'task',
			'goal',
		]);
	});

	it('selects an unselected kind when its chip is clicked', () => {
		setPage('/vault/all');

		const { getByRole } = render(FacetChips, { props: { facets: { task: 7, goal: 2 } } });
		getByRole('button', { name: /goal/ }).click();

		const target = new URL(String(goto.mock.calls[0][0]), 'http://localhost');
		expect(target.searchParams.getAll('doc_type_name')).toEqual(['goal']);
	});

	it('renders nothing at all when there is neither a histogram nor a selection', () => {
		setPage('/vault/all');

		const { queryAllByRole } = render(FacetChips, { props: { facets: null } });

		expect(queryAllByRole('button')).toEqual([]);
	});
});
