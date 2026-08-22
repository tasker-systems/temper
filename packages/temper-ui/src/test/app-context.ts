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
 * Only `page`, `navigating`, `goto` and `invalidateAll` are stubbed — the four the vault,
 * nav and layout components actually reach for. Add another when a component needs it,
 * rather than pre-building a full SvelteKit double.
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

/**
 * The `$navigating` fields the components under test read. SvelteKit's own `Navigation` is a
 * discriminated union over five navigation types; a component that only asks *is a navigation
 * happening, and where to* needs neither the union nor `from`.
 */
export interface TestNavigation {
	to?: { url: URL };
}

let navigation: TestNavigation | null = null;
const navSubscribers = new Set<(value: TestNavigation | null) => void>();

/**
 * `$navigating`, same hand-rolled shape as `page` above and for the same reason: `setNavigating`
 * is the control surface, so the store itself only has to be subscribable. `null` is idle —
 * SvelteKit's own value between navigations.
 */
export const navigating = {
	subscribe(run: (value: TestNavigation | null) => void): () => void {
		navSubscribers.add(run);
		run(navigation);
		return () => {
			navSubscribers.delete(run);
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
 * Point the navigating store at `target`, notifying anything already mounted. `null` puts the
 * app back at rest — nothing here runs SvelteKit's router, so a test says when a navigation is
 * in flight and when it has landed.
 */
export function setNavigating(target: TestNavigation | null): void {
	navigation = target;
	for (const notify of navSubscribers) notify(navigation);
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
	setNavigating(null);
}
