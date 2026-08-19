import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { columnsFor } from '$lib/vault-columns';
import { resetAppContext, setPage } from '../../test/app-context';
import { MANAGED, makeRow } from '../../test/fixtures';
import VaultGrid from './VaultGrid.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));
vi.mock('$app/navigation', () => import('../../test/app-context'));

beforeEach(resetAppContext);

/** One task row — the chrome the chip lives in only renders when the grid has rows. */
const ROW = makeRow({ title: 'A task', managed_meta: { ...MANAGED, 'temper-stage': 'design' } });

/** `kind` is what `VaultBrowser` reveals: `null` is the mixed set, with no stage column. */
function mount(kind: string | null) {
	return render(VaultGrid, {
		props: {
			rows: [ROW],
			columns: columnsFor(kind),
			total: 1,
			returned: 1,
			truncated: false,
			limit: 50,
			offset: 0,
		},
	});
}

/**
 * `orphanSort` itself is fully pinned by `vault-columns.test.ts` — the null cases, a sort field
 * the door rejects, the order defaulting, and the visible-column case. Nothing here re-asserts
 * any of it. What no pure test can see is the WIRING: `orphanSort` returning a value is not the
 * same as that value reaching the screen, and a clear link that computes the right href but is
 * never rendered leaves the reader with an ordering they can see the effect of and cannot name.
 */
describe('VaultGrid — a sort no visible column can mark', () => {
	it('names the orphaned sort in a chip', () => {
		// The repro from `orphanSort`'s own doc comment: sorted by Stage, then the `task` chip is
		// deselected, so the set goes mixed and `columnsFor(null)` emits no stage column at all.
		setPage('/vault/all?sort=stage&order=desc&offset=40');

		const { getByText } = mount(null);

		expect(getByText(/sorted by stage desc/)).toBeDefined();
	});

	it('clears sort, order and offset through the chip link', () => {
		setPage('/vault/all?doc_type_name=task&sort=stage&order=desc&offset=40');

		const { getByRole } = mount(null);
		const href = getByRole('link', { name: 'Clear sort' }).getAttribute('href') ?? '';
		const target = new URL(href, 'http://localhost');

		expect(target.searchParams.get('sort')).toBeNull();
		expect(target.searchParams.get('order')).toBeNull();
		// `offset` goes too: the first page of the old ordering is not the first page of the
		// new one, and landing on page 3 of a re-sorted list is a different set of rows.
		expect(target.searchParams.get('offset')).toBeNull();
		// Everything else survives — this clears a sort, not the filters.
		expect(target.searchParams.get('doc_type_name')).toBe('task');
	});

	it('shows no chip while the sorted column is on screen to carry the indicator', () => {
		setPage('/vault/all?doc_type_name=task&sort=stage&order=desc');

		const { queryByRole } = mount('task');

		// Same URL as above; only the revealed kind differs, so the stage column is present and
		// marks the sort itself. A chip here would name a state the header already shows.
		expect(queryByRole('link', { name: 'Clear sort' })).toBeNull();
	});
});
