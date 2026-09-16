import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetAppContext } from '../../test/app-context';
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

const answer = (status: number, body: unknown) =>
	vi.stubGlobal(
		'fetch',
		vi.fn(() => Promise.resolve(new Response(JSON.stringify(body), { status }))),
	);

/** Type a query and cross the 150ms debounce, draining the fetch it fires. */
const type = async (input: HTMLElement, q: string) => {
	await fireEvent.input(input, { target: { value: q } });
	await vi.advanceTimersByTimeAsync(150);
};

describe('CommandPalette — a search read that failed', () => {
	it('renders the failure, never "No results", when the endpoint answers 503', async () => {
		vi.useFakeTimers();
		// The laundered body the endpoint used to send: rows-shaped, non-ok status. The
		// client must distinguish on the status alone.
		answer(503, { rows: [], total: 0 });
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
		answer(503, { rows: [], total: 0 });
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

describe('CommandPalette — a search read that answered', () => {
	it('renders "No results" only for a successful empty answer', async () => {
		vi.useFakeTimers();
		answer(200, { rows: [], total: 0 });
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'zzz-nothing');

		expect(container.textContent).toContain('No results');
		expect(container.textContent).not.toContain('Search unavailable');
	});

	it('renders the rows a successful answer carried', async () => {
		vi.useFakeTimers();
		answer(200, { rows: [makeRow({ title: 'Ledger design' })], total: 1 });
		const { getByPlaceholderText, container } = await mountOpen();

		await type(getByPlaceholderText('Search the vault...'), 'ledger');

		expect(container.textContent).toContain('Ledger design');
		expect(container.textContent).not.toContain('No results');
	});
});
