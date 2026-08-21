import type { Anchor } from '$lib/graph/composition';
import type {
	CogmapAnalyticsRow,
	CogmapRegionMetricsRow,
	CogmapRegionRow,
	CogmapRow,
} from '$lib/types/generated/cognitive_maps';
import type { ContextRowWithCounts } from '$lib/types/generated/context';
import type { AtlasEntry, AtlasSubgraph } from '$lib/types/generated/graph_atlas';
import type { Composition, QueryResponse } from '$lib/types/generated/query';
import type { ContentResponse } from '$lib/types/generated/resource';
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

export const runComposition = (token: string, composition: Composition): Promise<QueryResponse> =>
	apiPost<QueryResponse>('/api/query', token, composition);

/**
 * The entry read — **what this reader's work is built around**, for a reader who asked nothing.
 *
 * This replaces `readSeedRows`, and the replacement is the whole point of the split. That function
 * paged the reader's rows by `updated DESC` — pure recency — while the walk seeded from every
 * visible resource, so the drawn set and the walked set were chosen by unrelated criteria and
 * **244 of 250 marks arrived with their edges dropped** for having an endpoint off-canvas.
 *
 * Here one criterion decides both: rank by degree, return the induced subgraph over the top of that
 * ranking. Every edge has both endpoints drawn by construction. Measured on production, the
 * unconnected band falls from 97.6% to 20%.
 *
 * `anchorIds` confines the ranking to the places named; empty ranks across the whole visible corpus.
 * The response carries its own bounds, because there is no composition trace here to borrow them
 * from.
 *
 * @see internal/superpowers/specs/2026-08-20-grounding-and-navigation-split-design.md §5.1
 */
export const readEntry = (token: string, anchorIds: string[]): Promise<AtlasEntry> => {
	const params = new URLSearchParams();
	if (anchorIds.length > 0) params.set('in', anchorIds.join(','));
	const qs = params.toString();
	return apiGet<AtlasEntry>(`/api/graph/entry${qs ? `?${qs}` : ''}`, token);
};

/**
 * The traversal read — **moving inside a space a question already set, without re-running it.**
 *
 * `[ruled — §10.3]` *"asking a question and our query composition frame helps set the space, but
 * then you traverse the graph as normal without a question locking you in."* So this runs no
 * composition, and the walk is deliberately **not confined to the grounding's result set**:
 * `traversal_slice` calls `graph_induced_edges` over the reader's whole visible corpus.
 *
 * **The seeds go over the wire comma-separated in ONE param, not as repeated ones.** The page
 * grammar spells them `?from=a&from=b` (`params.getAll('from')`) and this endpoint spells them
 * `?from=a,b` (`q.from.split(',')` in `handlers/graph.rs`). Two spellings of one list, and the join
 * is where they meet — passing the page's repeated form straight through would hand the service a
 * single unparseable uuid and 400.
 *
 * `depth` is omitted when the address carries none, so the default stays in one place: the handler
 * declares `depth: Option<i32>` and does `unwrap_or(1)`. The service clamps to `1..=3` regardless.
 *
 * **This endpoint has had no caller since chunk B landed it.** Chunk A shipped in the same shape —
 * green tests, zero callers — and three defects fell out the moment its output met a real server.
 *
 * @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §4
 */
export const traversePath = (seeds: string[], depth: number | null): string => {
	const params = new URLSearchParams({ from: seeds.join(',') });
	if (depth !== null) params.set('depth', String(depth));
	return `/api/graph/traverse?${params.toString()}`;
};

export const readTraversal = (
	token: string,
	seeds: string[],
	depth: number | null,
): Promise<AtlasSubgraph> => apiGet<AtlasSubgraph>(traversePath(seeds, depth), token);

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

/** The metrics door for an anchor — the analytics-tier sibling of {@link anchorShapePath}. */
const anchorMetricsPath = (anchor: Anchor): string =>
	anchor.kind === 'cogmap'
		? `/api/cognitive-maps/${anchor.id}/region-metrics`
		: `/api/contexts/${anchor.id}/region-metrics`;

/**
 * Everything the analysis door reads about one place. Four reads, none of them new.
 *
 * **The two per-region reads are the same pairing as `shape`** —
 * `/api/contexts/{id}/region-metrics` and `/api/cognitive-maps/{id}/region-metrics` are two doors
 * onto one `anchor_region_metrics_select(principal, HomeAnchor, lens)`
 * (`handlers/contexts.rs:282`, `handlers/cognitive_maps.rs:294`), both returning
 * `Vec<CogmapRegionMetricsRow>`. So the receiver needs no per-kind branch for the half that
 * carries what Beat B displaced.
 *
 * **No `lens` is passed**, for the same definitional reason `readAnchorRegions` passes none: the
 * lens is a clustering-time parameter, and naming one at read time would look up a different set
 * of regions than the place actually published.
 *
 * Three different failure postures, and the differences are the point:
 *
 * - **`shape` throws.** It is the row set; without it there is no page, and an empty list is
 *   already the honest answer for a place the caller cannot read (the API refuses to be an
 *   existence oracle).
 * - **`metrics` degrades to `null`.** That is *unknown*, not *absent* — captioning 501 groupings
 *   "not computed" on a read that never answered would be a claim about the substrate made on
 *   evidence the surface does not have.
 * - **`analytics` 404s to `null`.** A 404 here is a deny, and the task's own acceptance says it
 *   renders "not available" and never an error. Contexts are not asked at all: there is no context
 *   analytics read (D6 is unshipped), and inventing a peer field is exactly what the task forbids.
 */
export async function readAnchorAnalysis(
	token: string,
	anchor: Anchor,
): Promise<{
	shape: CogmapRegionRow[];
	metrics: CogmapRegionMetricsRow[] | null;
	analytics: CogmapAnalyticsRow | null;
	telos: ResourceView | null;
}> {
	const [shape, metrics, analytics] = await Promise.all([
		apiGet<CogmapRegionRow[]>(anchorShapePath(anchor), token),
		apiGet<CogmapRegionMetricsRow[]>(anchorMetricsPath(anchor), token).catch(() => null),
		anchor.kind === 'cogmap'
			? apiGet<CogmapAnalyticsRow>(`/api/cognitive-maps/${anchor.id}/analytics`, token).catch(
					() => null,
				)
			: Promise.resolve(null),
	]);

	// The charter's title, so the link says what it points at rather than showing a uuid. The
	// column is NOT NULL, so this is a read that should succeed — and a failure still leaves a
	// linkable id, which is why it degrades rather than throws.
	const telos = analytics
		? await apiGet<ResourceView>(`/api/resources/${analytics.telos_resource_id}`, token).catch(
				() => null,
			)
		: null;

	return { shape, metrics, analytics, telos };
}
