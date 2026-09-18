import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PAGE_SIZE, runSearch } from '$lib/server/vault-search';
import type { ExactArm, SearchResponse, WideArm } from '$lib/types/generated/search';
import { goto, resetAppContext } from '../../test/app-context';
import { makeRow } from '../../test/fixtures';
import CommandPalette from './CommandPalette.svelte';

vi.mock('$app/navigation', () => import('../../test/app-context'));

beforeEach(resetAppContext);
afterEach(() => {
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

/** The palette only renders while open; `toggle` is its export surface. */
const mountOpen = async () => {
	const rendered = render(CommandPalette);
	rendered.component.toggle();
	await tick();
	return rendered;
};

const answer = (status: number, body: unknown) => {
	// `init` is required on purpose: a palette that calls fetch without a request init is
	// not POSTing, and this mock is what says so.
	const fetchMock = vi.fn((_url: unknown, _init: RequestInit) =>
		Promise.resolve(new Response(JSON.stringify(body), { status })),
	);
	vi.stubGlobal('fetch', fetchMock);
	return fetchMock;
};

/** Type a query and cross the 150ms debounce, draining the fetch it fires. */
const type = async (input: HTMLElement, q: string) => {
	await fireEvent.input(input, { target: { value: q } });
	await vi.advanceTimersByTimeAsync(150);
};

const exactHit = (title: string): ExactArm['hits'][number] => ({
	resource: makeRow({ title }),
	fts_norm: 0.5,
});
const wideHit = (title: string): WideArm['hits'][number] => ({
	resource: makeRow({ title }),
	vec_norm: 0.9,
});

/**
 * The wire shape `POST /api/search` answers with on main: two arms, each keyed with its own
 * disposition, never combined. Both arms are always present — that is what makes the arm
 * selection display-side.
 */
const arms = ({
	exact = { hits: [], reason: 'no_match', hint: null },
	wide = { hits: [], reason: 'no_match', hint: null, degraded: false },
}: {
	exact?: Partial<ExactArm>;
	wide?: Partial<WideArm>;
} = {}): SearchResponse => ({
	exact: { hits: [], reason: 'no_match', hint: null, ...exact },
	wide: { hits: [], reason: 'no_match', hint: null, degraded: false, ...wide },
	scope: { kind: 'global', size: null },
});

describe('CommandPalette — which arm the answer is read from', () => {
	it('reads the wide arm by default and never merges the arms into one list', async () => {
		vi.useFakeTimers();
		answer(
			200,
			arms({
				exact: { hits: [exactHit('Exact-only find')], reason: 'ok', hint: null },
				wide: { hits: [wideHit('Wide-only find')], reason: 'ok', hint: null },
			}),
		);
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		// One act answering, visibly: the wide arm's rows, and NOT the exact arm's beside
		// them. A palette that merges both arms into a single ordered list renders
		// "Exact-only find" here and fails this.
		expect(container.textContent).toContain('Wide-only find');
		expect(container.textContent).not.toContain('Exact-only find');
		expect(container.textContent).toContain('similar in meaning');
		expect(container.textContent).not.toContain('matching these words');
	});

	it('the Exact switch selects the exact arm from the answer already returned — no second read', async () => {
		vi.useFakeTimers();
		const fetchMock = answer(
			200,
			arms({
				exact: { hits: [exactHit('Exact-only find')], reason: 'ok', hint: null },
				wide: { hits: [wideHit('Wide-only find')], reason: 'ok', hint: null },
			}),
		);
		const { getByPlaceholderText, getByLabelText, container } = await mountOpen();
		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		await fireEvent.click(getByLabelText('Exact'));
		await tick();

		expect(container.textContent).toContain('Exact-only find');
		expect(container.textContent).not.toContain('Wide-only find');
		expect(container.textContent).toContain('matching these words');
		// The arm was in the response; selecting it is a display move. A palette that
		// re-asks the door per mode pays a second read for an answer it already held.
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});
});

describe('CommandPalette — the wide arm could not run', () => {
	it("says so, and offers Exact as the reader's action — never auto-switches", async () => {
		vi.useFakeTimers();
		// The poisoned shape: the exact arm CARRIES hits, so an auto-switching palette would
		// render them and hide the failure. The switch stays off; the exact rows stay hidden.
		answer(
			200,
			arms({
				exact: { hits: [exactHit('Exact-only find')], reason: 'ok', hint: null },
				wide: { hits: [], reason: 'no_match', hint: 'embedding unavailable', degraded: true },
			}),
		);
		const { getByPlaceholderText, getByLabelText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		expect(container.textContent).toContain("Meaning-match couldn't run just now");
		expect(container.textContent).toContain('Exact matches words directly');
		// A degraded arm is not a learned emptiness: it must not render as "nothing matched"
		// either — nothing about the corpus was discovered.
		expect(container.textContent).not.toContain('Nothing matched');
		expect(container.textContent).not.toContain('No results');
		expect(container.textContent).not.toContain('Exact-only find');
		expect((getByLabelText('Exact') as HTMLInputElement).checked).toBe(false);
	});

	it('flips only when the reader clicks the offered action', async () => {
		vi.useFakeTimers();
		answer(
			200,
			arms({
				exact: { hits: [exactHit('Exact-only find')], reason: 'ok', hint: null },
				wide: { hits: [], reason: 'no_match', hint: 'embedding unavailable', degraded: true },
			}),
		);
		const { getByPlaceholderText, getByText, container } = await mountOpen();
		await type(getByPlaceholderText('Search the vault...'), 'ledger');
		expect(container.textContent).toContain("Meaning-match couldn't run just now");

		// The reader's explicit action — the one thing that is allowed to move the switch.
		await fireEvent.click(getByText('Exact matches words directly — use it'));
		await tick();

		expect(container.textContent).toContain('Exact-only find');
	});
});

describe('CommandPalette — an answered arm that found nothing', () => {
	it('no_match invites rephrasing, never renders as "No results"', async () => {
		vi.useFakeTimers();
		answer(200, arms({ wide: { hits: [], reason: 'no_match', hint: null } }));
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'zzz-nothing');
		expect(container.textContent).toContain('Nothing matched — try different words');
		expect(container.textContent).not.toContain('No results');
	});

	it('out_of_scope says rephrasing will never help, never renders as "No results"', async () => {
		vi.useFakeTimers();
		answer(200, arms({ wide: { hits: [], reason: 'out_of_scope', hint: null } }));
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'zzz-nothing');
		expect(container.textContent).toContain('No phrasing will reach this');
		expect(container.textContent).not.toContain('No results');
		expect(container.textContent).not.toContain('Nothing matched');
	});
});

describe('CommandPalette — the bound it states (D6)', () => {
	it('states the bound under a full page', async () => {
		vi.useFakeTimers();
		answer(
			200,
			arms({
				wide: {
					hits: Array.from({ length: PAGE_SIZE }, (_, i) => wideHit(`Find ${i}`)),
					reason: 'ok',
					hint: null,
				},
			}),
		);
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		// The answer may be truncated at the page size; a palette that hides that fact
		// reads as complete what is a first page.
		expect(container.textContent).toContain(`Showing the first ${PAGE_SIZE}`);
		expect(container.textContent).not.toContain('See all');
	});

	it('states no bound under a partial page — everything is there', async () => {
		vi.useFakeTimers();
		answer(200, arms({ wide: { hits: [wideHit('Find one')], reason: 'ok', hint: null } }));
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		expect(container.textContent).not.toContain('Showing the first');
	});

	it('the bare-Enter hand-off to the list door is gone', async () => {
		vi.useFakeTimers();
		answer(200, arms({ wide: { hits: [wideHit('Find one')], reason: 'ok', hint: null } }));
		const { getByPlaceholderText } = await mountOpen();
		const input = getByPlaceholderText('Search the vault...');
		await type(input, 'ledger');

		// Enter past the last row — focus moved beyond the hits, exactly where the old
		// palette handed off to the title-contains list door. Routing there would be the
		// substitution defect restated as a route; the palette keeps the reader here.
		await fireEvent.keyDown(input, { key: 'ArrowDown' });
		await fireEvent.keyDown(input, { key: 'ArrowDown' });
		await fireEvent.keyDown(input, { key: 'Enter' });
		expect(goto).not.toHaveBeenCalled();
	});
});

describe('CommandPalette — a question the door cannot pose (D7)', () => {
	it('declines a several-questions input saying so, and reads nothing upstream', async () => {
		vi.useFakeTimers();
		const fetchMock = answer(200, arms());
		const { getByPlaceholderText, container } = await mountOpen();

		await type(
			getByPlaceholderText('Search the vault...'),
			'how do I migrate the schema? and where do seeds live?',
		);

		expect(container.textContent).toContain('One question at a time');
		expect(fetchMock).not.toHaveBeenCalled();
	});

	it('does not decline one question, however it is punctuated', async () => {
		vi.useFakeTimers();
		const fetchMock = answer(200, arms());
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'search and rescue??');

		// "and" is not a question separator, and double punctuation is style — both are one
		// question, and the door is asked. A decline rule wider than this would refuse
		// answerable questions.
		expect(container.textContent).not.toContain('One question at a time');
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});
});

describe('CommandPalette — the request it makes', () => {
	it('posts the question to the proxy and nothing but the question', async () => {
		vi.useFakeTimers();
		const fetchMock = answer(200, arms());
		const { getByPlaceholderText } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		const [url, init] = fetchMock.mock.calls[0];
		expect(url).toBe('/_internal/search');
		expect(init.method).toBe('POST');
		// No `arms` key, no embedding — byte-identical to any current client's request.
		// The arm selection never reaches the wire.
		expect(JSON.parse(String(init.body))).toEqual({ q: 'ledger' });
		// The proxy module still owns the upstream shape.
		expect(runSearch).toBeDefined();
	});

	it('renders the failure, never "No results", when the endpoint answers 503', async () => {
		vi.useFakeTimers();
		answer(503, { error: 'search unavailable' });
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		expect(container.textContent).toContain('Search unavailable');
		expect(container.textContent).not.toContain('No results');
	});

	it('renders the failure when the request itself throws', async () => {
		vi.useFakeTimers();
		vi.stubGlobal(
			'fetch',
			vi.fn(() => Promise.reject(new Error('network down'))),
		);
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		expect(container.textContent).toContain('Search unavailable');
		expect(container.textContent).not.toContain('No results');
	});

	it('recovers to a neutral state when a later query is cleared', async () => {
		vi.useFakeTimers();
		answer(503, { error: 'search unavailable' });
		const { getByPlaceholderText, container } = await mountOpen();
		const input = getByPlaceholderText('Search the vault...');

		await type(input, 'ledger');
		expect(container.textContent).toContain('Search unavailable');

		// A fresh query re-arms the read: the failure belongs to the answer that failed,
		// not to the palette forever.
		await type(input, '');
		expect(container.textContent).not.toContain('Search unavailable');
	});
});
