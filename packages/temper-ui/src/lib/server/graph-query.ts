import type { SeedAxis } from '$lib/graph/bound';
import type { Anchor } from '$lib/graph/composition';
import type { Composition, QueryResponse } from '$lib/types/generated/query';
import type { ResourceListResponse } from '$lib/types/generated/resource';
import type { ResourceView } from '$lib/types/generated/resource_view';
import { apiGet, apiPost } from './api';

/**
 * The successor graph surface's reads.
 *
 * Deliberately **not** in `graph-reads.ts`: that file holds the nine bespoke `/api/graph/*`
 * endpoints the predecessor uses, which lose their only caller when this surface replaces it. Two
 * files means the deletion is a file rather than an excavation.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §0, §2
 */

/**
 * How many of the reader's own rows a no-question entry draws.
 *
 * The walk contributes at most 50 (`follow-from`'s published ceiling) and a question entry's survey
 * arm measured ~110 against production, so this is the same order of magnitude rather than a number
 * chosen for roundness — the screen stays in the low hundreds by every door.
 *
 * It is a page, not a cap that hides anything: the read reports its own `total`, so the bound line
 * states both halves and a reader is told what they are not seeing. Reading to exhaustion instead
 * would put 2,066 marks on the canvas for one ordinary context.
 */
export const SEED_ROWS = 200;

/** Never let a wide fan-out shave a place down to nothing; better to exceed SEED_ROWS slightly. */
const MIN_ROWS_PER_PLACE = 20;

export const runComposition = (token: string, composition: Composition): Promise<QueryResponse> =>
	apiPost<QueryResponse>('/api/query', token, composition);

const listPath = (limit: number, filter?: [string, string]): string => {
	const params = new URLSearchParams({ limit: String(limit) });
	if (filter) params.set(filter[0], filter[1]);
	return `/api/resources?${params}`;
};

/**
 * The reader's own rows in the places they asked about — the seeds a walk grows from but never
 * returns (`follow-from` walks *"at least one hop"*, so a seed is not in its own answer).
 *
 * **The unaddressed door reads with no filter at all**, and that is exact rather than lazy: a
 * resource is homed by exactly one anchor, so "every visible resource" IS the union of every
 * readable anchor. One read, and its `total` is a true denominator across all of them. Filtering
 * would produce the same set through N calls.
 *
 * A named set needs one read per context — `context_ref` takes one ref, not a list — plus one read
 * covering every named cogmap, since `cogmap_ids` is a CSV. Summing their totals stays exact for
 * the same reason: the anchors are disjoint, so no resource is counted twice.
 */
export async function readSeedRows(
	token: string,
	anchors: Anchor[],
	addressed: boolean,
): Promise<{ rows: ResourceView[]; axis: SeedAxis }> {
	const cogmapIds = anchors.filter((a) => a.kind === 'cogmap').map((a) => a.id);
	const contexts = anchors.filter((a) => a.kind === 'context');

	const filters: ([string, string] | undefined)[] = addressed
		? [
				...contexts.map((c): [string, string] => ['context_ref', c.ref]),
				...(cogmapIds.length > 0 ? [['cogmap_ids', cogmapIds.join(',')] as [string, string]] : []),
			]
		: [undefined];

	const perRead = Math.max(MIN_ROWS_PER_PLACE, Math.floor(SEED_ROWS / Math.max(filters.length, 1)));
	const pages = await Promise.all(
		filters.map((f) => apiGet<ResourceListResponse>(listPath(perRead, f), token)),
	);

	return {
		rows: pages.flatMap((p) => p.rows),
		axis: {
			shown: pages.reduce((n, p) => n + p.rows.length, 0),
			total: pages.reduce((n, p) => n + Number(p.total), 0),
			truncated: pages.some((p) => p.truncated),
		},
	};
}

/**
 * The rows for explicitly named `from` seeds.
 *
 * Reading them is what makes a stale seed an **honest 404 about the reader's own material** rather
 * than a silently smaller graph: `/api/resources/{id}` answers 404 for a resource that is gone or
 * was never readable, and the two are deliberately indistinguishable. Inferring it from the walk's
 * `input_unusable` instead would give the same number with none of the recourse.
 *
 * They are also needed for the drawing, for the same reason the seed rows above are: the walk
 * excludes what it grew from.
 */
export const readSeedResources = (token: string, ids: string[]): Promise<ResourceView[]> =>
	Promise.all(ids.map((id) => apiGet<ResourceView>(`/api/resources/${id}`, token)));
