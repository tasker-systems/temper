import { omissionSentence } from '@tasker-systems/temper-design/omission';
import { WAY_IN_BOUND, WAY_IN_ORDER, WAYS_IN } from '@tasker-systems/temper-design/ways-in';
import { render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	type FirstViewList,
	PLACES_ORDER,
	PLACES_SCOPE,
	RECENT_SCOPE,
	RESUME_LABEL,
	type Resume,
} from '$lib/first-view';
import { describeFailure, GaveUp, GIVE_UP_AFTER_MS } from '$lib/server/bounded';
import type { ContextActivity, ContextRowWithCounts } from '$lib/types';
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

const placeId = (i: number) => `0000000${i}-0000-7000-8000-00000000000c`;

/** The places the layout read: named, in the nav's order. */
const contextsOf = (n: number): ContextRowWithCounts[] =>
	Array.from(
		{ length: n },
		(_, i) =>
			({
				id: placeId(i),
				name: `place ${i}`,
				slug: `place-${i}`,
				owner_ref: '+team',
			}) as unknown as ContextRowWithCounts,
	);

/** The reader's change in each of the first `n` places, place 0 most recently. */
const activityIn = (n: number): ContextActivity[] =>
	Array.from({ length: n }, (_, i) => ({
		context_id: placeId(i),
		last_changed_at: new Date(Date.now() - (i + 1) * 3_600_000).toISOString(),
	})) as ContextActivity[];

function dataWith(
	over: Partial<Record<'resume' | 'opening' | 'recent' | 'places', Promise<unknown>>> & {
		contexts?: ContextRowWithCounts[] | null;
		ways?: Promise<FirstViewList>[];
	},
): PageData {
	return {
		resume: Promise.resolve(null),
		opening: Promise.resolve(null),
		recent: Promise.resolve(list(5, 40, 'recent')),
		places: Promise.resolve(activityIn(2)),
		contexts: contextsOf(2),
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
	it('is not the all-resources grid', async () => {
		const { container } = await mount(dataWith({}));
		expect(container.querySelector('table')).toBeNull();
		expect(container.querySelector('h1')?.textContent).toBe('Home');
	});

	// The latest handoff told a browsing reader nothing: handoffs are written by agents for the next
	// agent. It is gone from the first view, under its name and under any other.
	it('draws no handoff', async () => {
		const { container } = await mount(dataWith({}));
		expect(container.textContent).not.toMatch(/handoff|next steps/i);
		expect(container.querySelector('#handoff-heading')).toBeNull();
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

	it('says a given-up resume gave up, and still draws the ways in', async () => {
		const { container, getByTestId } = await mount(dataWith({ resume: gaveUpOn(RESUME_LABEL) }));
		expect(container.querySelector('[data-region="gave-up"]')?.textContent).toContain(RESUME_LABEL);
		expect(getByTestId(`way-${WAYS_IN[0].id}`).querySelectorAll('li')).toHaveLength(2);
	});
});

describe('recently updated, failing alone', () => {
	it('fails without touching the ways in', async () => {
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

describe('the places nearest are the ones in use', () => {
	it("names each from the layout's contexts, orders by the reader's change, and says by what", async () => {
		// The layout's order is the nav's (place 0 … 6); the reader changed place 3 most recently.
		const activity = activityIn(7).reverse();
		activity[3] = { ...activity[3], last_changed_at: new Date().toISOString() };
		const { getByTestId } = await mount(
			dataWith({ places: Promise.resolve(activity), contexts: contextsOf(7) }),
		);
		const region = getByTestId('places');
		expect([...region.querySelectorAll('li a')].map((a) => a.textContent?.trim())).toEqual([
			'place 3',
			'place 0',
			'place 1',
			'place 2',
			'place 4',
		]);
		expect(region.querySelector('li a')?.getAttribute('href')).toBe('/vault/+team/place-3');
		expect(region.querySelector('[data-testid="place-changed"]')?.textContent?.trim()).toBe(
			'you changed just now',
		);
		const foot = region.querySelector('[data-testid="omission"]')?.textContent;
		expect(foot).toContain(omissionSentence(7, 5, PLACES_SCOPE));
		expect(foot).toContain(`Ordered ${PLACES_ORDER}.`);
	});

	it('says the reader has changed nothing yet, rather than drawing an empty list', async () => {
		const { getByTestId } = await mount(dataWith({ places: Promise.resolve([]) }));
		const region = getByTestId('places');
		expect(region.querySelector('[data-testid="region-empty"]')).not.toBeNull();
		expect(region.querySelector('ul')).toBeNull();
		expect(region.querySelector('[data-testid="omission"]')).toBeNull();
	});

	it('fails alone: a failed places read names itself and leaves the other regions standing', async () => {
		const { getByTestId } = await mount(dataWith({ places: broken() }));
		expect(getByTestId('places').textContent).toContain(
			'Places you changed something in unavailable',
		);
		expect(getByTestId('recent').querySelectorAll('li')).toHaveLength(5);
	});

	it('fails, rather than claiming none, when the layout could not read the places', async () => {
		const { getByTestId } = await mount(dataWith({ contexts: null }));
		const region = getByTestId('places');
		expect(region.querySelector('[data-testid="region-failed"]')).not.toBeNull();
		expect(region.querySelector('[data-testid="region-empty"]')).toBeNull();
	});
});

describe('work resumes where it was left', () => {
	const changedAt = new Date(Date.now() - 3 * 3_600_000).toISOString();
	const LAST: Resume = {
		resource: makeRow({
			id: '01a17626-b8cf-74e0-9f36-d0ec473d7128',
			title: 'The work I left',
			doc_type_name: 'task',
			// A teammate edited it since; that time must not be the one shown.
			updated: new Date().toISOString(),
			changed_by_me_at: changedAt,
		}),
		changedAt,
	};

	it('offers, first, one step back to the work, with the time the reader changed it', async () => {
		const { getByTestId, container } = await mount(dataWith({ resume: Promise.resolve(LAST) }));
		const region = getByTestId('resume');
		expect(region.querySelector('h2')?.textContent?.trim()).toBe(RESUME_LABEL);
		const links = region.querySelectorAll('a');
		expect(links).toHaveLength(1);
		expect(links[0].getAttribute('href')).toBe(`/vault/r/${LAST.resource.id}`);
		expect(links[0].textContent).toBe('The work I left');
		expect(region.querySelector('[data-testid="resume-changed"]')?.textContent?.trim()).toBe(
			'task · you changed 3h ago',
		);
		// First: the region right after the header, before the places.
		expect(container.querySelector('header')?.nextElementSibling).toBe(region);
	});

	it('renders the opening of the work, and says when it continues', async () => {
		const { getByTestId } = await mount(
			dataWith({
				resume: Promise.resolve(LAST),
				opening: Promise.resolve({ markdown: 'Ship **the return**.', truncated: true }),
			}),
		);
		const opening = getByTestId('resume-opening');
		await waitFor(() => expect(opening.querySelector('.md-body')).not.toBeNull());
		expect(opening.querySelector('strong')?.textContent).toBe('the return');
		expect(opening.textContent).toContain('… continues in the task.');
	});

	it('says nothing continues when the opening is the whole of it', async () => {
		const { getByTestId } = await mount(
			dataWith({
				resume: Promise.resolve(LAST),
				opening: Promise.resolve({ markdown: 'All of it.', truncated: false }),
			}),
		);
		const opening = getByTestId('resume-opening');
		await waitFor(() => expect(opening.querySelector('.md-body')).not.toBeNull());
		expect(opening.textContent).toContain('All of it.');
		expect(opening.textContent).not.toContain('continues');
	});

	it('shows the return without an opening when the work has no prose', async () => {
		const { getByTestId } = await mount(dataWith({ resume: Promise.resolve(LAST) }));
		expect(getByTestId('resume').querySelector('[data-testid="resume-opening"]')).toBeNull();
		expect(getByTestId('resume').querySelector('[data-region]')).toBeNull();
	});

	it('fails the opening alone, keeping the return', async () => {
		const { getByTestId } = await mount(
			dataWith({ resume: Promise.resolve(LAST), opening: broken() }),
		);
		const region = getByTestId('resume');
		expect(region.querySelector('[data-region="failed"]')?.textContent).toContain('Opening');
		expect(region.querySelector(`a[href="/vault/r/${LAST.resource.id}"]`)).not.toBeNull();
	});

	it('offers no return, and draws nothing in its place, when the reader has changed nothing', async () => {
		const { queryByTestId, container } = await mount(dataWith({ resume: Promise.resolve(null) }));
		expect(queryByTestId('resume')).toBeNull();
		expect(container.textContent).not.toMatch(new RegExp(`\\b${RESUME_LABEL}\\b`, 'i'));
		// No stand-in under any name: the places follow the header directly, and with every other
		// read answered, no region on the page draws a state.
		expect(container.querySelector('header')?.nextElementSibling).toBe(queryByTestId('places'));
		expect(container.querySelectorAll('[data-region]')).toHaveLength(0);
	});

	it('fails alone, keeping the places', async () => {
		const { getByTestId } = await mount(dataWith({ resume: broken() }));
		expect(getByTestId('resume').querySelector('[data-region="failed"]')).not.toBeNull();
		expect(getByTestId('places').querySelectorAll('li')).toHaveLength(2);
	});
});
