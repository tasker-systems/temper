/**
 * The SvelteKit ambient modules a component test has to stand in for, plus the control
 * surface a test drives them with. One module serves both roles on purpose: a test file
 * mocks with `() => import('...')` and then imports the same module directly, so the
 * component under test and the assertions are looking at the same instance.
 *
 * ```ts
 * vi.mock('$app/stores', () => import('../../test/app-context'));
 * vi.mock('$app/navigation', () => import('../../test/app-context'));
 * import { goto, resetAppContext, setPage } from '../../test/app-context';
 * ```
 *
 * Only `page`, `goto` and `invalidateAll` are stubbed — the three the vault and nav
 * components actually reach for. Add another when a component needs it, rather than
 * pre-building a full SvelteKit double.
 */
import { vi } from 'vitest';

/** The `$page` fields the components under test read. Not the full SvelteKit shape. */
export interface TestPage {
	url: URL;
	params: Record<string, string>;
}

export const ORIGIN = 'http://localhost';

let current: TestPage = { url: new URL('/', ORIGIN), params: {} };
const subscribers = new Set<(value: TestPage) => void>();

/**
 * A minimal readable store. Hand-rolled rather than `svelte/store`'s `writable` so that
 * `setPage` can take a href and params — the two things a test actually varies — instead
 * of asking every caller to assemble the page object.
 */
export const page = {
	subscribe(run: (value: TestPage) => void): () => void {
		subscribers.add(run);
		run(current);
		return () => {
			subscribers.delete(run);
		};
	},
};

export const goto = vi.fn((_url: string | URL): Promise<void> => Promise.resolve());
export const invalidateAll = vi.fn((): Promise<void> => Promise.resolve());

/**
 * Point the page store at `href`, notifying anything already mounted. Route params are
 * supplied explicitly because nothing here runs SvelteKit's router — a component reading
 * `$page.params.owner` is reading what the test set, and the test says so.
 */
export function setPage(href: string, params: Record<string, string> = {}): void {
	current = { url: new URL(href, ORIGIN), params };
	for (const notify of subscribers) notify(current);
}

/**
 * The URL of the nth `goto` call, parsed. Every test in the "click that mutates the URL"
 * category asserts on a navigation target's `searchParams` or `pathname`, so this is that,
 * once, rather than a `new URL(String(goto.mock.calls[n][0]), …)` rebuilt per file.
 *
 * There is deliberately no `currentPage()` counterpart: `goto` is a no-op mock and never
 * moves the page store, so "did the component navigate" is answered by asserting on `goto`
 * itself (`expect(goto).not.toHaveBeenCalled()`), never by reading back a URL.
 */
export function gotoTarget(call = 0): URL {
	return new URL(String(goto.mock.calls[call][0]), ORIGIN);
}

/** Call from `beforeEach`. Mock call history survives module state otherwise. */
export function resetAppContext(): void {
	goto.mockClear();
	invalidateAll.mockClear();
	setPage('/');
}
