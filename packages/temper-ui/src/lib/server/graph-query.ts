import type { Anchor } from '$lib/graph/composition';
import type {
	AnchorShape,
	CogmapAnalyticsRow,
	CogmapRegionMetricsRow,
	CogmapRegionRow,
	CogmapRow,
	CogmapStaleness,
	ShapeEmptiness,
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
 * @see temper-artifacts:specs/2026-08-20-graph-successor-surface-design.md §0, §2
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
 * @see temper-artifacts:specs/2026-08-20-grounding-and-navigation-split-design.md §5.1
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
 * @see temper-artifacts:specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §4
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
 * `anchor_shape_select(principal, HomeAnchor, lens)` and both return `AnchorShape` — the region
 * rows inside an anchor-level envelope (`population`, `emptiness`, `materialized_at`) that lets an
 * empty answer say WHY it is empty rather than arriving as a byte-identical `[]`. So
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
 *
 * **The envelope is unwrapped here, with ONE exception that is the whole point of the exception.**
 * This lookup exists to NAME a disclosed region id, and `nameOf` matches on `region_id` alone, so
 * `population` and `materialized_at` are dropped: they are per-anchor facts that would have to be
 * re-associated with the anchors they came from to say anything, and the flat set below is
 * deliberately not keyed that way.
 *
 * **`emptiness` is read, and not to report a cause.** `complete` means *a read did not answer*, and
 * it used to be `every(fulfilled)` — which is an HTTP-level question asked of an authorization-level
 * failure. A caller who may not read an anchor gets `emptiness: 'unreadable_or_absent'` with
 * `population: 0` **on a 200, never a 403** (`substrate_read.rs`, *"discloses strictly less than a
 * 403 would, and stays a 200"*) — deliberately, so the shape read is not an existence oracle. So a
 * denial arrived *fulfilled*, `complete` stayed `true`, and `nameOf` answered `re-derived`:
 *
 *     a region the trace disclosed, whose anchor this caller cannot read
 *       →  200, zero rows  →  complete: true  →  "This grouping has been re-derived."
 *
 * which is a claim about the substrate drawn from a read that told the caller nothing. `unchecked`
 * exists for exactly that — *"the surface must never tell a reader their grouping is gone on
 * evidence it does not have"* (`readout.ts`) — and was unreachable in the one case it was built for,
 * because the posture that protects the anchor also disguises the denial as an empty success.
 *
 * **This does not collapse `complete` into an `emptiness` arm.** The two still mean different
 * things, and only ONE arm is consulted: `never_clustered` and `nothing_visible` are answers — the
 * anchor genuinely holds nothing for this caller, and an unfound id really is `re-derived`.
 * `unreadable_or_absent` is not an answer about the anchor at all; it is the read declining to make
 * one. Reading it here asks the question `complete` always asked, at the layer that can answer it.
 */
export async function readAnchorRegions(
	token: string,
	anchors: Anchor[],
): Promise<{ rows: CogmapRegionRow[]; complete: boolean }> {
	const reads = await Promise.allSettled(
		anchors.map((a) => apiGet<AnchorShape>(anchorShapePath(a), token)),
	);

	/** A rejection did not answer — and neither did a 200 that declined to say anything. */
	const answered = (r: PromiseSettledResult<AnchorShape>): boolean =>
		r.status === 'fulfilled' && r.value.emptiness !== 'unreadable_or_absent';

	return {
		// Unchanged: a denied read carries no rows anyway, so this only ever drops empties.
		rows: reads.flatMap((r) => (r.status === 'fulfilled' ? r.value.regions : [])),
		complete: reads.every(answered),
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
 * **It resolves to a string, never to `null`.** `[corrected — 2026-08-21]` This returned
 * `Promise<string | null>` under a comment claiming *"`content.markdown` is nullable on the wire"*.
 * It is not: `crates/temper-workflow/src/types/resource.rs:426` is `pub markdown: String` and the
 * generated `src/lib/types/generated/resource.ts:18` is `markdown: string`. The `| null` was left
 * behind when the `.catch(() => null)` below was deleted, and it made the type say a failure could
 * still arrive as a value — the very reading this amendment removed. A resource with **no body**
 * arrives as an empty string, and `excerptOf` is what turns that into the `empty` state.
 *
 * **A failure rejects.** `[amended — 2026-08-21, spec §5.2]` This used to `.catch(() => null)`, on
 * the reasoning that a rail which cannot show an excerpt still shows the resource. That reasoning
 * is right and survives; its *target* was wrong. Degrading to `null` here made a failed read
 * indistinguishable from a resource with no body — spec §5.1 names this as one of the live
 * instances — and it did so inside the reader, where the caller has no way to tell them apart
 * again. The caller streams this promise and renders the rejection as a **named failure**, which
 * is the degradation the policy actually asks for.
 */
export const readResourceBody = (token: string, id: string): Promise<string> =>
	apiGet<ContentResponse>(`/api/resources/${id}/content`, token).then((r) => r.markdown);

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

/** The anchor-level door — the third of the three that come in a cogmap/context pair. */
const anchorAnalyticsPath = (anchor: Anchor): string =>
	anchor.kind === 'cogmap'
		? `/api/cognitive-maps/${anchor.id}/analytics`
		: `/api/contexts/${anchor.id}/analytics`;

/**
 * What the anchor-level door answers — **two shapes, and the difference between them is the
 * answer** rather than a gap in it.
 *
 * `/api/cognitive-maps/{id}/analytics` answers with the clock **plus a charter resource and a
 * regulation set**; `/api/contexts/{id}/analytics` answers with the clock alone. A context has no
 * charter and no regulation set, so those two are not fields it declines to fill — they are fields
 * it cannot have. Widening this into one type with both of them optional would spell *nothing
 * found* about two things that **cannot exist**, which is exactly the faked peer field
 * `CONTEXT_HAS_NO_MAP_READOUT` was written to refuse (`$lib/graph/analysis.ts`).
 *
 * So it is a union discriminated on the anchor kind, and the two halves are carried differently on
 * purpose:
 *
 * - the cogmap arm **intersects the generated wire type** rather than restating its fields, so a
 *   change to `CogmapAnalyticsRow` lands here by construction and cannot drift;
 * - `staleness` sits on **both** members, so the half the two anchor kinds genuinely share reads
 *   without narrowing, and the half they do not share cannot be read without it.
 */
export type AnchorAnalytics =
	| ({ kind: 'cogmap' } & CogmapAnalyticsRow)
	| { kind: 'context'; staleness: CogmapStaleness };

/**
 * Everything the analysis door reads about one place. Four reads.
 *
 * **`[2026-08-25]` One of them is new for one anchor kind.** `/api/contexts/{id}/analytics` did not
 * exist when this door was built, so contexts were skipped and the page declared the readout absent
 * for them. It exists now; they are asked. The other three are unchanged.
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
 * - **`shape` throws.** It is the row set; without it there is no page, and an empty envelope is
 *   already the honest answer for a place the caller cannot read (the API refuses to be an
 *   existence oracle — a denied caller gets `emptiness: 'unreadable_or_absent'` with
 *   `population: 0` on a 200, never a 403; `substrate_read.rs:1308-1312`).
 * - **`metrics` degrades to `null`.** That is *unknown*, not *absent* — captioning 501 groupings
 *   "not computed" on a read that never answered would be a claim about the substrate made on
 *   evidence the surface does not have.
 * - **`analytics` 404s to `null`.** A 404 here is a deny, and the task's own acceptance says it
 *   renders "not available" and never an error. **Both anchor kinds are asked** — the context door
 *   `/api/contexts/{id}/analytics` ships as of `context_analytics` — and they are asked for
 *   different shapes, because a context is answered for staleness and gets **no** charter and
 *   **no** regulation. Inventing either as a null peer field is exactly what the task forbids: see
 *   {@link AnchorAnalytics}, which is a union rather than one optional-everything row for that
 *   reason alone.
 */
export async function readAnchorAnalysis(
	token: string,
	anchor: Anchor,
): Promise<{
	shape: CogmapRegionRow[];
	emptiness: ShapeEmptiness | null;
	metrics: CogmapRegionMetricsRow[] | null;
	analytics: AnchorAnalytics | null;
	telos: ResourceView | null;
}> {
	// The kind is decided ONCE, here, and travels on the value. The alternative — re-deriving it
	// downstream from which fields happen to be present — would make the presence of a charter the
	// definition of being a map, and a map whose read was declined would then read as a context.
	const analyticsRead: Promise<AnchorAnalytics | null> =
		anchor.kind === 'cogmap'
			? apiGet<CogmapAnalyticsRow>(anchorAnalyticsPath(anchor), token)
					.then((row): AnchorAnalytics => ({ kind: 'cogmap', ...row }))
					.catch(() => null)
			: apiGet<CogmapStaleness>(anchorAnalyticsPath(anchor), token)
					.then((staleness): AnchorAnalytics => ({ kind: 'context', staleness }))
					.catch(() => null);

	const [shape, metrics, analytics] = await Promise.all([
		apiGet<AnchorShape>(anchorShapePath(anchor), token),
		apiGet<CogmapRegionMetricsRow[]>(anchorMetricsPath(anchor), token).catch(() => null),
		analyticsRead,
	]);

	// The charter's title, so the link says what it points at rather than showing a uuid. The
	// column is NOT NULL, so this is a read that should succeed — and a failure still leaves a
	// linkable id, which is why it degrades rather than throws.
	//
	// Only a map has one to fetch. A context is not asked for a charter it does not have — the
	// narrowing is what stops this from becoming a lookup of `undefined`.
	const telos =
		analytics?.kind === 'cogmap'
			? await apiGet<ResourceView>(`/api/resources/${analytics.telos_resource_id}`, token).catch(
					() => null,
				)
			: null;

	// **`emptiness` is carried; the rest of the envelope is still dropped here.** This door is the
	// one place a PERSON meets an empty region set, and until this field crossed it the page said
	// "This place has no groupings yet." for all four causes -- asserting `never_clustered` on a
	// read that may have meant any of them. That is the same claim-a-cause-you-cannot-know defect
	// `16a9e357` fixed at the CLI door, and this is its last unfixed instance.
	//
	// **`population` is deliberately NOT carried, and the reason is arithmetic rather than taste.**
	// It is the all-lens denominator, and this door passes no `lens` (see above -- the lens is a
	// clustering-time parameter). With `p_lens IS NULL` the shape function's row filter and its
	// `population` count range over the same `regs` set, so `population === shape.regions.length`
	// on every read this door can make. Surfacing it would print the row count twice under two
	// names. **`lens_narrowed` is unreachable here for the same reason**: arm 3 fires only when
	// `regs` is non-empty while the lens-filtered rows are empty, which no NULL lens can produce
	// (`migrations/20260823000010_anchor_shape_envelope.sql:121-122` and `:132`). The receiver still
	// that arm, because the type has four and an exhaustive match is what keeps a future
	// lens-passing caller honest -- but it is labelled unreachable rather than left to look live.
	//
	// **`materialized_at` is not carried either.** The page already shows a clock for maps, from
	// `analytics.staleness`; adding a second one from a different read would put two timestamps
	// about one place on one page with nothing saying why they differ. It is also stamped at the
	// materialize transaction's START rather than at the end of the clustering work, so it runs
	// systematically early -- a skew worth fixing before it is put in front of a reader, not after.
	return { shape: shape.regions, emptiness: shape.emptiness, metrics, analytics, telos };
}
