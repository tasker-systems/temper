import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { VaultList } from '$lib/vault-list';
import { invalidateAll, resetAppContext, setPage } from '../../../test/app-context';
import { MANAGED, makeRow } from '../../../test/fixtures';
import VaultBrowser from './VaultBrowser.svelte';

vi.mock('$app/stores', () => import('../../../test/app-context'));
vi.mock('$app/navigation', () => import('../../../test/app-context'));

beforeEach(resetAppContext);

/** One row, only so the success arm has a grid with something in it to render. */
const ROW = makeRow({ title: 'A task', managed_meta: { ...MANAGED, 'temper-stage': 'design' } });

const LIST: VaultList = {
	rows: [ROW],
	total: 1,
	returned: 1,
	truncated: false,
	limit: 50,
	offset: 0,
	facets: { doc_type: { task: 1 }, stage: {}, status: {} },
};

function mount(list: VaultList | null, loadError: string | null = null) {
	return render(VaultBrowser, {
		props: { title: 'All resources', list, loadError, contexts: [] },
	});
}

/**
 * `toVaultList` and the `VaultList | null` shape are pinned by `vault-list.test.ts`, and
 * `activeFilterCount` by `vault-filters.test.ts`. Neither can see the branch: a component that
 * built the right caption and rendered the grid anyway — from `list?.rows ?? []` — is green in
 * both, and puts an empty grid in front of a reader whose read FAILED. The two halves are
 * asserted together on purpose; "the alert is present" alone is satisfied by a component that
 * renders both, which is the exact defect the `VaultList | null` shape exists to prevent.
 */
describe('VaultBrowser — a failed read renders the error state instead of the grid', () => {
	it('renders the alert, and no grid, when the read failed', () => {
		setPage('/vault/all');

		const { container, getByRole } = mount(null, 'The vault could not be read.');

		expect(getByRole('alert')).toBeDefined();
		// `.vault-grid-wrapper` is `VaultGrid`'s own root, present whether the grid has rows or
		// renders its empty state — so this fails on either way of rendering it alongside.
		expect(container.querySelector('.vault-grid-wrapper')).toBeNull();
	});

	it('claims no count anywhere on a failed read', () => {
		setPage('/vault/all');

		const { queryByText } = mount(null, 'The vault could not be read.');

		// A caption is always a count here, and "0 resources" on a failed read is a lie the
		// reader cannot tell from an honest empty page.
		expect(queryByText(/\d+\s+(matching\s+)?resources?/)).toBeNull();
	});

	it('renders the grid, and no alert, on a successful read', () => {
		setPage('/vault/all');

		const { container, queryByRole } = mount(LIST);

		expect(container.querySelector('.vault-grid-wrapper')).not.toBeNull();
		expect(queryByRole('alert')).toBeNull();
	});
});

/**
 * The one control the error state offers. `invalidateAll` re-runs the failed `load`, which is
 * the only way out of this state without a manual reload — a button wired to nothing looks
 * identical on screen and leaves the reader stranded.
 */
describe('VaultBrowser — the retry control', () => {
	it('re-runs the load when Try again is clicked', () => {
		setPage('/vault/all');

		const { getByRole } = mount(null, 'The vault could not be read.');
		getByRole('button', { name: 'Try again' }).click();

		expect(invalidateAll).toHaveBeenCalledTimes(1);
	});
});
