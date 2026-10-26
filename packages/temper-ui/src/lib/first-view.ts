/**
 * The web's first view: where a signed-in reader arrives (`/vault`), oriented from their own work
 * rather than from a grid of everything they can see (`arrival-starts-from-the-work`).
 *
 * It holds what can be read today without recording anything about the reader:
 *
 * - **the latest handoff** — their most recently updated session, and the next steps it
 *   recorded, quoted;
 * - **the standing ways in** — the reader's own active goals, tasks in progress and sessions,
 *   declared once in `@tasker-systems/temper-design/ways-in`, so the web offers each under the
 *   desktop's name, scope, bound and order (`the-same-ways-in-on-both-surfaces`);
 * - **the places you work in** — the contexts the reader changed something in, ordered by their own
 *   latest change in each (`places-nearest-are-the-ones-in-use`). This ordering is the first
 *   view's alone: the nav keeps its stable order (`no-nav-place-moves-with-activity`);
 * - **recently updated** — across everything the reader can read, and it says so.
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
import type { ContextActivity, ContextRowWithCounts, ResourceView } from '$lib/types';

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

/** The reader's own most recently updated session. */
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

/** The reader's own latest change, place by place. */
export function placesPath(): string {
	return '/api/me/context-activity';
}

/** How many places are shown — the same bound as recently updated. */
export const PLACES_BOUND = RECENT_BOUND;

/** What the places list covers, as its omission sentence names it. */
export const PLACES_SCOPE = 'places you changed something in';

/** What the places list is ordered by, said where the list is. */
export const PLACES_ORDER = 'by your latest change in each';

/** One place as the first view shows it: the context, and when the reader last changed it there. */
export type PlaceInUse = { context: ContextRowWithCounts; lastChangedAt: string };

/** The places list: its rows, and how many places the reader has changed something in. */
export type PlacesList = { rows: PlaceInUse[]; total: number };

/**
 * The places the reader works in, newest change of their own first.
 *
 * The order is the reader's `last_changed_at` and nothing else — not the context's `updated`,
 * which a teammate's write moves, and not its name. The read is already newest first; it is sorted
 * again here so the order the page states is one this function guarantees, not one it inherits.
 *
 * The activity read carries no names, so each place takes its name and address from the contexts
 * the layout already read. A place missing from them is dropped: both reads are gated by what the
 * reader can read, so a miss is only a race between them, and an unnamed row could not be followed.
 * Contexts that were never read (`null`) leave nothing to name the places with, and that is the
 * region's failure, not an empty list.
 */
export function placesInUse(
	activity: ContextActivity[],
	contexts: ContextRowWithCounts[] | null,
	bound: number = PLACES_BOUND,
): PlacesList {
	if (contexts === null) throw new Error('the places you can read were not read');
	const byId = new Map(contexts.map((c) => [c.id, c]));
	const all = activity
		.flatMap((a) => {
			const context = byId.get(a.context_id);
			return context ? [{ context, lastChangedAt: a.last_changed_at }] : [];
		})
		.sort((a, b) => Date.parse(b.lastChangedAt) - Date.parse(a.lastChangedAt));
	return { rows: all.slice(0, bound), total: all.length };
}
