import { HANDOFF_LABEL } from '@tasker-systems/temper-design/handoff';
import { omissionSentence } from '@tasker-systems/temper-design/omission';
import { WAY_IN_BOUND, WAY_IN_ORDER, WAYS_IN } from '@tasker-systems/temper-design/ways-in';
import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type FirstViewList, RECENT_SCOPE } from '$lib/first-view';
import { describeFailure, GaveUp, GIVE_UP_AFTER_MS } from '$lib/server/bounded';
import { resetAppContext, setPage } from '../../../test/app-context';
import { makeRow } from '../../../test/fixtures';
import Page from './+page.svelte';
import type { PageData } from './$types';

vi.mock('$app/stores', () => import('../../../test/app-context'));
afterEach(() => resetAppContext());

/**
 * The first view as a reader meets it. `page.server.test.ts` beside this file witnesses which
 * reads the load makes; this witnesses what each region says once they answer — that the ways in
 * carry the shared names and say what they omit and how they are ordered, and that one region's
 * failure reads as that region's failure, never as another's absence.
 */

const broken = <T>(): Promise<T> => {
	const p = Promise.reject<T>(new Error('503'));
	p.catch(() => {});
	return p;
};

/** A read the system stopped waiting for, in the shape one really arrives in (see `$lib/region`). */
const gaveUpOn = <T>(label: string): Promise<T> => {
	const p = Promise.reject<T>(
		describeFailure(new GaveUp(label, GIVE_UP_AFTER_MS), 'Internal Error'),
	);
	p.catch(() => {});
	return p;
};

const list = (n: number, total = n, prefix = 'row'): FirstViewList => ({
	rows: Array.from({ length: n }, (_, i) =>
		makeRow({ id: `0000000${i}-0000-7000-8000-000000000000`, title: `${prefix} ${i}` }),
	),
	total,
});

const SESSION = makeRow({ id: '01a17104-f5db-7782-b326-70ce7109c2d5', title: 'The last session' });

function dataWith(
	over: Partial<Record<'session' | 'next' | 'recent', Promise<unknown>>> & {
		ways?: Promise<FirstViewList>[];
	},
): PageData {
	return {
		session: Promise.resolve(SESSION),
		next: Promise.resolve({
			heading: 'Next Steps',
			text: 'Ship the first view.',
			truncated: false,
		}),
		recent: Promise.resolve(list(5, 40, 'recent')),
		...over,
		ways: WAYS_IN.map((way, i) => ({
			way,
			shown: WAY_IN_BOUND,
			list: over.ways?.[i] ?? Promise.resolve(list(2, 2, way.id)),
		})),
	} as unknown as PageData;
}

async function mount(data: PageData) {
	setPage('/vault');
	const view = render(Page, { data });
	await new Promise((r) => setTimeout(r, 0));
	await tick();
	return view;
}

describe('arrival starts from the work', () => {
	it("quotes the latest handoff's next steps as written, under the shared name", async () => {
		const { container, getByTestId } = await mount(dataWith({}));
		expect(container.textContent).toContain(HANDOFF_LABEL);
		expect(container.querySelector(`a[href="/vault/r/${SESSION.id}"]`)?.textContent).toBe(
			'The last session',
		);
		expect(getByTestId('next-steps').querySelector('blockquote')?.textContent?.trim()).toBe(
			'Ship the first view.',
		);
	});

	it('says the handoff is the reader’s own, and how recently its session was updated', async () => {
		const { container, getByTestId } = await mount(dataWith({}));
		expect(container.querySelector('#handoff-heading')?.textContent?.trim()).toBe(
			`your ${HANDOFF_LABEL}`,
		);
		expect(getByTestId('handoff-age').textContent).toMatch(/^\s*session · updated \S/);
	});

	it('quotes nothing when the session recorded no next steps', async () => {
		const { queryByTestId, container } = await mount(dataWith({ next: Promise.resolve(null) }));
		expect(queryByTestId('next-steps')).toBeNull();
		expect(container.querySelector(`a[href="/vault/r/${SESSION.id}"]`)).not.toBeNull();
	});

	it('says there is no session yet, rather than showing an empty box', async () => {
		const { container } = await mount(
			dataWith({ session: Promise.resolve(null), next: Promise.resolve(null) }),
		);
		expect(container.querySelector('[data-region="empty"]')?.textContent).toContain(
			'sessions recorded yet',
		);
	});

	it('is not the all-resources grid', async () => {
		const { container } = await mount(dataWith({}));
		expect(container.querySelector('table')).toBeNull();
		expect(container.querySelector('h1')?.textContent).toBe('Home');
	});
});

describe('the same ways in on both surfaces', () => {
	it('offers every shared way in under its name, saying what it omits and its order', async () => {
		const { getByTestId } = await mount(
			dataWith({
				ways: [Promise.resolve(list(8, 12)), Promise.resolve(list(3)), Promise.resolve(list(1))],
			}),
		);
		const totals = [12, 3, 1];
		const shown = [8, 3, 1];
		WAYS_IN.forEach((way, i) => {
			const region = getByTestId(`way-${way.id}`);
			expect(region.querySelector('h2')?.textContent).toBe(way.label);
			expect(region.querySelector('[data-testid="omission"]')?.textContent).toContain(
				omissionSentence(totals[i], shown[i], way.scope),
			);
			expect(region.textContent).toContain(`Ordered ${WAY_IN_ORDER}.`);
		});
	});

	it('offers the rest of a way in by growing it in place, never by opening the grid', async () => {
		const { getByTestId } = await mount(dataWith({ ways: [Promise.resolve(list(8, 12))] }));
		const more = getByTestId(`way-${WAYS_IN[0].id}`).querySelector('a[href^="/vault?"]');
		expect(more?.textContent?.trim()).toBe('Show 4 more');
		expect(more?.getAttribute('href')).toBe(`/vault?${WAYS_IN[0].id}=${WAY_IN_BOUND * 2}`);
		expect(getByTestId(`way-${WAYS_IN[0].id}`).querySelector('a[href^="/vault/all"]')).toBeNull();
	});

	it('offers no more when every row is shown', async () => {
		const { getByTestId } = await mount(dataWith({ ways: [Promise.resolve(list(3))] }));
		expect(getByTestId(`way-${WAYS_IN[0].id}`).querySelector('a[href^="/vault?"]')).toBeNull();
	});

	it('names an empty way in by what it holds', async () => {
		const { getByTestId } = await mount(dataWith({ ways: [Promise.resolve(list(0))] }));
		const region = getByTestId(`way-${WAYS_IN[0].id}`);
		expect(region.querySelector('[data-region="empty"]')?.textContent).toContain(WAYS_IN[0].scope);
	});
});

describe('a failed read is its own region’s failure', () => {
	it('fails one way in and leaves the others drawn', async () => {
		const { getByTestId } = await mount(dataWith({ ways: [broken()] }));
		expect(
			getByTestId(`way-${WAYS_IN[0].id}`).querySelector('[data-region="failed"]'),
		).not.toBeNull();
		expect(getByTestId(`way-${WAYS_IN[1].id}`).querySelector('[data-region]')).toBeNull();
	});

	it('says a given-up handoff gave up, and still draws the ways in', async () => {
		const { container, getByTestId } = await mount(dataWith({ session: gaveUpOn(HANDOFF_LABEL) }));
		expect(container.querySelector('[data-region="gave-up"]')?.textContent).toContain(
			HANDOFF_LABEL,
		);
		expect(getByTestId(`way-${WAYS_IN[0].id}`).querySelectorAll('li')).toHaveLength(2);
	});

	it('fails the next steps alone, keeping the session it belongs to', async () => {
		const { container } = await mount(dataWith({ next: broken() }));
		expect(container.querySelector('[data-region="failed"]')?.textContent).toContain(
			'Next steps unavailable',
		);
		expect(container.querySelector(`a[href="/vault/r/${SESSION.id}"]`)).not.toBeNull();
	});
});

describe('recently updated, failing alone', () => {
	it('fails without touching the handoff or the ways in', async () => {
		const { getByTestId } = await mount(dataWith({ recent: broken() }));
		expect(getByTestId('recent').querySelector('[data-region="failed"]')).not.toBeNull();
		expect(getByTestId(`way-${WAYS_IN[0].id}`).querySelectorAll('li')).toHaveLength(2);
	});
});

describe('recently updated', () => {
	it('says it covers everything the reader can read, and its order', async () => {
		const { getByTestId } = await mount(dataWith({}));
		const region = getByTestId('recent');
		expect(region.textContent).toContain(omissionSentence(40, 5, RECENT_SCOPE));
		expect(region.textContent).toContain(`Ordered ${WAY_IN_ORDER}.`);
	});
});
