/**
 * The web's first view: where a signed-in reader arrives (`/vault`), oriented from their own work
 * rather than from a grid of everything they can see (`arrival-starts-from-the-work`).
 *
 * It holds what can be read today without recording anything about the reader:
 *
 * - **resume** — first, the one resource the reader most recently changed, when they changed it,
 *   and the opening of it (`work-resumes-where-it-was-left`). The resource and its time come from
 *   the list door narrowed to the reader's own changes, so the time is theirs and never `updated`,
 *   which a teammate's write moves. A reader who has changed nothing is offered no return, and
 *   nothing stands in its place. A record of what the reader opened, or the resources they own,
 *   would look like it and answer a different question — both are ruled out;
 * - **the standing ways in** — the reader's own active goals, tasks in progress and sessions,
 *   declared once in `@tasker-systems/temper-design/ways-in`, so the web offers each under the
 *   desktop's name, scope, bound and order (`the-same-ways-in-on-both-surfaces`);
 * - **the places you work in** — the contexts the reader changed something in, ordered by their own
 *   latest change in each (`places-nearest-are-the-ones-in-use`). This ordering is the first
 *   view's alone: the nav keeps its stable order (`no-nav-place-moves-with-activity`);
 * - **recently updated** — across everything the reader can read, and it says so.
 *
 * The latest handoff is not here. It was, and seen in a browser it told a reader nothing: handoffs
 * are written by agents for the next agent, and agents do not browse the web.
 *
 * Every list says the order it is read in: the ways in and recently updated by newest update, the
 * places and resume by the reader's own latest change. The paths here are pure so a test can pin
 * exactly what each list asks the door for.
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

/**
 * The resource the reader most recently changed, by their own change time — the order the list door
 * gives `sort=changed_by_me`, which it accepts only under `changed_by=@me`.
 */
export function resumePath(): string {
	const params = new URLSearchParams({
		changed_by: '@me',
		sort: 'changed_by_me',
		order: 'desc',
		limit: '1',
	});
	return `/api/resources?${params}`;
}

/** What resume is called, in its heading and in every state of its region. */
export const RESUME_LABEL = 'resume';

/** The return resume offers: the resource, and when the reader last changed it. */
export type Resume = { resource: ResourceView; changedAt: string };

/**
 * The return, from the first row of the resume read; `null` when the reader has changed nothing.
 *
 * A row without its change time is a broken read, not a return without a time: showing `updated`
 * in its place would claim a recency the reader may not hold, and dropping the row would read as
 * the reader having changed nothing. It fails the region instead.
 */
export function resumeOf(rows: ResourceView[]): Resume | null {
	const resource = rows[0];
	if (!resource) return null;
	if (!resource.changed_by_me_at) throw new Error('the time of your change was not returned');
	return { resource, changedAt: resource.changed_by_me_at };
}

/** How much of the resumed work's body is shown before it says it continues. */
export const OPENING_BOUND = 400;

/** The opening resume shows: whole blocks of the body as written, and whether more follows. */
export type Opening = { markdown: string; truncated: boolean };

/** An ATX heading, empty ones included, indented at most three spaces. */
const HEADING = /^ {0,3}#{1,6}(\s|$)/;
/** A setext underline: it makes the paragraph above it a heading. */
const UNDERLINE = /^ {0,3}(=+|-+)\s*$/;
/** A fence's marker. Only a run of the same character, at least as long, closes it. */
const FENCE = /^ {0,3}(`{3,}|~{3,})/;

/**
 * The opening of a body: its first stretch of prose, past any headings it starts with, up to the
 * next heading. Whole blocks are kept while they fit the bound, so the markdown stays well formed;
 * only a first block longer than the bound is cut, at a word. Quoted as written, never summarised,
 * and `null` for a body with no prose, so nothing is invented in its place.
 */
export function openingOf(markdown: string, bound: number = OPENING_BOUND): Opening | null {
	const lines = markdown.split(/\r?\n/);
	const blocks: string[][] = [];
	let block: string[] = [];
	let fence: string | null = null;
	let i = 0;
	for (; i < lines.length; i++) {
		const line = lines[i];
		const marker = FENCE.exec(line);
		if (fence) {
			block.push(line);
			const closes =
				marker &&
				marker[1][0] === fence[0] &&
				marker[1].length >= fence.length &&
				!line.slice(marker[0].length).trim();
			if (closes) fence = null;
			continue;
		}
		if (marker) {
			fence = marker[1];
			block.push(line);
			continue;
		}
		const atx = HEADING.test(line);
		if (atx || (block.length && UNDERLINE.test(line))) {
			if (!atx) block = [];
			// Headings the body starts with are skipped; the first one after its prose ends it.
			if (blocks.length || block.length) break;
			continue;
		}
		if (!line.trim()) {
			if (block.length) blocks.push(block);
			block = [];
			continue;
		}
		block.push(line);
	}
	if (block.length) blocks.push(block);
	if (!blocks.length) return null;
	const rest = lines.slice(i + 1).some((l) => l.trim() && !HEADING.test(l) && !UNDERLINE.test(l));

	const first = blocks[0].join('\n');
	if (first.length > bound) {
		const cut = first.slice(0, bound);
		const atWord = cut
			.slice(0, Math.max(cut.lastIndexOf(' '), bound - 40))
			.replace(/[\uD800-\uDBFF]$/, '');
		return { markdown: `${atWord.trimEnd()}…`, truncated: true };
	}
	const kept = [first];
	let length = first.length;
	for (const next of blocks.slice(1)) {
		const text = next.join('\n');
		if (length + 2 + text.length > bound) break;
		kept.push(text);
		length += 2 + text.length;
	}
	return { markdown: kept.join('\n\n'), truncated: rest || kept.length < blocks.length };
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
