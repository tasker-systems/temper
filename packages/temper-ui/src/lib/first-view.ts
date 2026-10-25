/**
 * The web's first view: where a signed-in reader arrives (`/vault`), oriented from their own work
 * rather than from a grid of everything they can see (`arrival-starts-from-the-work`).
 *
 * It holds what can be read today without recording anything about the reader:
 *
 * - **the latest handoff** — their newest session, and the next steps it recorded, quoted;
 * - **the standing ways in** — the reader's own active goals, tasks in progress and sessions,
 *   declared once in `@tasker-systems/temper-design/ways-in`, so the web offers each under the
 *   desktop's name, scope, bound and order (`the-same-ways-in-on-both-surfaces`);
 * - **recently updated** — across everything the reader can read, and it says so. The web has no
 *   signal for where a reader works, so it does not claim to be scoped to it.
 *
 * **Resume is not here, and nothing stands in its place.** It means the work the reader last
 * *changed*, and no read answers that yet; it lands beside the handoff when one does. A record of
 * what the reader opened, or the resources they own, would look like it and answer a different
 * question — both are ruled out.
 *
 * Every list is read newest update first, and the page says so. The paths here are pure so a test
 * can pin exactly what each list asks the door for.
 */

import { WAY_IN_BOUND, type WayIn } from '@tasker-systems/temper-design/ways-in';
import type { ResourceView } from '$lib/types';

/** One bounded list as the first view shows it: its rows, and how many there are in all. */
export type FirstViewList = { rows: ResourceView[]; total: number };

/** How many recently updated rows are shown — the desktop's bound for the same section. */
export const RECENT_BOUND = 5;

/** What recently updated covers, as its omission sentence names it. */
export const RECENT_SCOPE = 'resources you can read';

/** Where a signed-in reader arrives: sign-in, and anything else that lands them, comes here. */
export const ARRIVAL = '/vault';

const NEWEST_FIRST = { sort: 'updated', order: 'desc' } as const;

/** The most a way in grows to on this page; past it, the reader is in the grid's territory. */
export const WAY_IN_MAX = 200;

/**
 * How many rows a way in shows, from its own URL parameter (`?goals=16`). Each list grows in place
 * by the shared step, as the desktop's "Show more" does — never by opening the grid, which would
 * carry the `owner` narrowing without saying so. Anything unreadable is the default bound.
 */
export function wayInShown(params: URLSearchParams, way: WayIn): number {
	const asked = Number(params.get(way.id));
	if (!Number.isInteger(asked) || asked <= WAY_IN_BOUND) return WAY_IN_BOUND;
	return Math.min(asked, WAY_IN_MAX);
}

/** The first view with one way in grown by a step, every other list as it was. */
export function wayInMoreHref(url: URL, way: WayIn, shown: number): string {
	const params = new URLSearchParams(url.searchParams);
	params.set(way.id, String(Math.min(shown + WAY_IN_BOUND, WAY_IN_MAX)));
	return `${url.pathname}?${params}`;
}

/** What a way in asks the list door for: its filter, its bound, newest update first. */
export function wayInPath(way: WayIn, bound: number): string {
	const params = new URLSearchParams({
		doc_type_name: way.filter.docType,
		owner: way.filter.owner,
	});
	if (way.filter.status) params.set('status', way.filter.status);
	if (way.filter.stage) params.set('stage', way.filter.stage);
	params.set('limit', String(bound));
	params.set('sort', NEWEST_FIRST.sort);
	params.set('order', NEWEST_FIRST.order);
	return `/api/resources?${params}`;
}

/** The newest session the reader themselves wrote. */
export function handoffPath(): string {
	const params = new URLSearchParams({
		doc_type_name: 'session',
		owner: '@me',
		limit: '1',
		...NEWEST_FIRST,
	});
	return `/api/resources?${params}`;
}

/** Recently updated, across everything the reader can read. */
export function recentPath(bound: number = RECENT_BOUND): string {
	const params = new URLSearchParams({ limit: String(bound), ...NEWEST_FIRST });
	return `/api/resources?${params}`;
}
