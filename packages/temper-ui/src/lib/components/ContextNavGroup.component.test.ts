import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NavGroup } from '$lib/nav-groups';
import type { ContextRowWithCounts } from '$lib/types';
import { resetAppContext, setPage } from '../../test/app-context';
import ContextNavGroup from './ContextNavGroup.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));
vi.mock('$app/navigation', () => import('../../test/app-context'));

beforeEach(resetAppContext);

function ctx(slug: string, ownerRef: string, count = 0): ContextRowWithCounts {
	return {
		id: `ctx-${slug}`,
		name: slug,
		kb_owner_table: 'kb_teams',
		kb_owner_id: 'team-id',
		created: '2026-01-01T00:00:00Z',
		updated: '2026-01-01T00:00:00Z',
		// The wire delivers a JSON number; ts-rs declares `bigint`. Same cast
		// `nav-groups.test.ts` makes, for the same reason — but narrowed: this fixture is
		// team-owned unconditionally, because the component never reads the owner columns.
		resource_count: count as unknown as bigint,
		slug,
		owner_ref: ownerRef,
	};
}

/** One team group holding two places — the shape `navContextsState` emits. */
const GROUP: NavGroup = {
	key: '+platform',
	label: 'Platform',
	kind: 'team',
	contexts: [ctx('infra', '+platform', 7), ctx('runbooks', '+platform', 4)],
	resourceCount: 11,
};

const LIST_SELECTOR = '[id="nav-group-+platform"]';

/** The heading's collapsed-only mark, keyed off the one string it says to a reader. */
const MARK = 'You are in this group';

function mount(collapsed: boolean, onToggle: () => void = () => {}) {
	return render(ContextNavGroup, { props: { group: GROUP, collapsed, onToggle } });
}

/** The reader is standing in `+platform/infra` — a place this group holds. */
function standInGroup(): void {
	setPage('/vault/+platform/infra', { owner: '+platform', context: 'infra' });
}

/** The reader is standing somewhere else entirely — no place in this group is lit. */
function standElsewhere(): void {
	setPage('/vault/+research/papers', { owner: '+research', context: 'papers' });
}

/**
 * The persisted preference itself is pinned by `stores/sidebar.test.ts` (`parseCollapsedGroups`,
 * `toggleCollapsedGroup`, `defaultCollapsed`), and the predicate that decides whether a place is
 * the reader's location by `vault-url.test.ts` (`isContextLocation`). Neither can see the join:
 * `collapsed` arrives as a prop and `isContextLocation` reads real route params, so what a group
 * DOES when both are true is a rendering fact that only exists once they meet in the component.
 *
 * `/dev/nav` cannot see it either, and says so: nothing there supplies route params, so no place
 * is ever lit in the harness. The heading mark is prod-verified exactly once, by hand. This is
 * the seam the component layer exists for.
 */
describe('ContextNavGroup — a collapsed group holding the reader’s place', () => {
	it('stays collapsed rather than re-opening itself', () => {
		standInGroup();

		const { container, getByRole } = mount(true);

		// Forcing the group open here would make a control labelled "Collapse" do nothing.
		// The preference stays authoritative; the mark below is what keeps the place legible.
		expect(container.querySelector(LIST_SELECTOR)?.hasAttribute('hidden')).toBe(true);
		expect(getByRole('button').getAttribute('aria-expanded')).toBe('false');
	});

	it('marks the heading, so the reader has not lost where they are', () => {
		standInGroup();

		const { getByTitle } = mount(true);

		expect(getByTitle(MARK)).toBeDefined();
	});

	it('reports the group’s own total, in the heading, whether shut or open', () => {
		standInGroup();

		// The count sits unconditionally in the heading — there is no `collapsed` branch on
		// it — so both mounts are asserted, and the assertion is on the count element rather
		// than a substring of the whole heading, where "11" would also match a place's count.
		for (const collapsed of [true, false]) {
			const { getByRole, unmount } = mount(collapsed);
			const spans = getByRole('button').querySelectorAll('span');
			expect(spans[spans.length - 1].textContent?.trim()).toBe('11');
			unmount();
		}
	});
});

/**
 * The conjunct that proves the mark tracks the reader's PLACE, not merely `collapsed`. A heading
 * that marks every collapsed group, or that marks while the lit place is already on screen, says
 * nothing — and both are green against the two tests above on their own.
 */
describe('ContextNavGroup — where the mark must NOT appear', () => {
	it('shows no mark on a collapsed group the reader is not standing in', () => {
		standElsewhere();

		const { container, queryByTitle } = mount(true);

		expect(container.querySelector(LIST_SELECTOR)?.hasAttribute('hidden')).toBe(true);
		expect(queryByTitle(MARK)).toBeNull();
	});

	it('shows no mark while expanded, where the lit place already says it', () => {
		standInGroup();

		const { queryByTitle } = mount(false);

		expect(queryByTitle(MARK)).toBeNull();
	});

	it('shows its places when expanded', () => {
		standInGroup();

		const { container, getByRole } = mount(false);

		expect(container.querySelector(LIST_SELECTOR)?.hasAttribute('hidden')).toBe(false);
		expect(getByRole('button').getAttribute('aria-expanded')).toBe('true');
	});
});

/**
 * `toggleCollapsedGroup` is pure-tested; that the disclosure is wired to it is not. A heading
 * that computes the next preference and never hands it back leaves a control that visibly does
 * nothing, and no pure test can tell the difference.
 */
describe('ContextNavGroup — the disclosure control', () => {
	it('calls onToggle when the heading is clicked', () => {
		standInGroup();

		const onToggle = vi.fn();
		const { getByRole } = mount(true, onToggle);
		getByRole('button').click();

		expect(onToggle).toHaveBeenCalledTimes(1);
	});
});
