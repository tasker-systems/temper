import type { SeedAxis } from '$lib/graph/bound';
import type { Anchor } from '$lib/graph/composition';
import type { CogmapRegionRow, CogmapRow } from '$lib/types/generated/cognitive_maps';
import type { ContextRowWithCounts } from '$lib/types/generated/context';
import type { Composition, QueryResponse } from '$lib/types/generated/query';
import type { ContentResponse, ResourceListResponse } from '$lib/types/generated/resource';
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

/**
 * One anchor's regions, for naming the groupings the readout discloses.
 *
 * **Both anchor kinds answer the same read**, and that is not a coincidence to be tidied away:
 * `/api/contexts/{id}/shape` and `/api/cognitive-maps/{id}/shape` are two doors onto one
 * `anchor_shape_select(principal, HomeAnchor, lens)` and both return `Vec<CogmapRegionRow>`. So
 * `cross-kind-relationship-is-reachable` holds a layer below the composition too, and the readout
 * needs no per-kind branch to name what it drew on. **Contexts genuinely have regions** — measured,
 * `@me/temper` holds 499 — so resolving only cogmaps would render every context-anchored grouping
 * as *re-derived*, which is precisely the false alarm the clause forbids.
 *
 * **No `lens` is passed, and that is definitional rather than a default.** `survey` calls
 * `wayfind_region_scores` with `p_lens = NULL` — *"the lens is a clustering-time parameter; NULL
 * reads the baked salience"* — so naming one here would look up a different set of regions than
 * the ones the answer actually drew on, and disagree with the trace by construction.
 *
 * **The cost, stated rather than glossed:** one read per ASKED anchor — not per disclosing one —
 * and each returns that anchor's WHOLE shape, because no door offers a subset by id. Measured on
 * the heaviest real reader: 12 parallel reads returning 983 rows in total, and the caller skips
 * the whole thing when nothing was disclosed. Narrowing to just the anchors that disclosed would
 * mean pairing survey stages to anchors by list index, which holds today only by construction; the
 * saving did not look worth a correctness trap that nothing would catch when it broke. That is the
 * price of naming a grouping at all — the alternative is a readout that can only count.
 */
const anchorShapePath = (anchor: Anchor): string =>
	anchor.kind === 'cogmap'
		? `/api/cognitive-maps/${anchor.id}/shape`
		: `/api/contexts/${anchor.id}/shape`;

/**
 * Resolve the anchors' regions, reporting whether the gathering was COMPLETE.
 *
 * A read that does not answer must not turn every unfound id into a claim that the grouping is
 * gone, so a rejection degrades the whole lookup to incomplete rather than propagating as a page
 * error. One anchor being unreadable is not a reason to refuse the reader their graph, and it is
 * not evidence that anything was re-derived either.
 */
export async function readAnchorRegions(
	token: string,
	anchors: Anchor[],
): Promise<{ rows: CogmapRegionRow[]; complete: boolean }> {
	const reads = await Promise.allSettled(
		anchors.map((a) => apiGet<CogmapRegionRow[]>(anchorShapePath(a), token)),
	);

	return {
		rows: reads.flatMap((r) => (r.status === 'fulfilled' ? r.value : [])),
		complete: reads.every((r) => r.status === 'fulfilled'),
	};
}

/**
 * The selected resource's body — one read, for one node, on demand.
 *
 * `GET /api/resources/{id}` takes **no** section parameter (it has no `Query` extractor at all),
 * so a body cannot ride on the row; `/{id}/content` is the door. That is why N1's excerpt is a
 * targeted read rather than a projection widened across the canvas: `list` deliberately refuses
 * `body` because *"a page of reconstructed bodies is unbounded"*, and this surface would be asking
 * for up to two hundred of them.
 *
 * Returns `null` rather than throwing: a rail that cannot show an excerpt still shows the
 * resource, and a body read failing is not a reason to fail the page.
 */
export const readResourceBody = (token: string, id: string): Promise<string | null> =>
	apiGet<ContentResponse>(`/api/resources/${id}/content`, token)
		.then((r) => r.markdown)
		.catch(() => null);

/**
 * Every anchor the reader can read — the input to `readableAnchors`.
 *
 * Both reads are **self-scoped**, which is what makes an absent place indistinguishable from a
 * nonexistent one at this door: `/api/contexts` goes through `context_visible_to` and
 * `/api/cognitive-maps` through `cogmap_visible_maps`, each returning exactly what the caller may
 * see and an empty list on deny.
 *
 * A failure **throws** rather than degrading to `[]`, and that is the opposite of the app shell's
 * choice for the same read — deliberately. There, an empty list degrades a filter bar. Here it
 * would silently become *"you have no places"*, and the unaddressed door would then draw nothing
 * while the bound line truthfully reported `0 of 0 places`: a well-formed, plausible, wrong answer.
 */
export const readAnchorSources = (token: string): Promise<[ContextRowWithCounts[], CogmapRow[]]> =>
	Promise.all([
		apiGet<ContextRowWithCounts[]>('/api/contexts', token),
		apiGet<CogmapRow[]>('/api/cognitive-maps', token),
	]);
