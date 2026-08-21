import type { Extent, QueryResponse, StageResult } from '$lib/types/generated/query';
import type { GraphPlan } from './composition';

/**
 * The bound declaration — always on screen, plain, never dismissible.
 *
 * **Four axes, and they know four different amounts about themselves.** Two come from the
 * composition, where the surface's whole job is to **stop discarding what the response already
 * discloses**. One — the places asked — the surface must declare from its own record, because no
 * `Extent` can ride on a truncation that happened before the composition existed. And the fourth,
 * the seed arm, comes from a list read that reports a real `total`, which is why it is the only one
 * that states a denominator.
 *
 * **No denominator is invented for any axis that lacks one.** `StageResult.total` is `None`
 * unconditionally (`temper-services/src/backend/query_read.rs:582`) — *"the fragments return a page,
 * not a count. Absent rather than guessed from the page size."* The clause this serves never required
 * a denominator; it requires that a partial view not be indistinguishable from a complete one, and
 * `Extent` draws exactly that line truthfully. Where a source genuinely HAS a denominator, refusing
 * to state it would be the same failure in the other direction.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §3
 */

/**
 * How wide the funnel actually ran, or why there is no number.
 *
 * Three states, not two, and they must never render alike: an entry that ran no funnel at all
 * (`applicable: false`) is a different fact from one that ran one and disclosed no width
 * (`applied: null`), and both are different from a width.
 */
export type GroupingsAxis = { applicable: false } | { applicable: true; applied: number | null };

/**
 * The seed arm of a no-question entry — the reader's own material in the places they named.
 *
 * `follow-from` walks *"at least one hop"*, so the seeds are **not** in the walked arm; they come
 * from the list read the table half already makes. That read is the only source on this screen with
 * a **true denominator**: `ResourceListResponse.total` is *"the FILTERED match count — every row the
 * filters admit, before `limit`/`offset`"*, and `truncated` is *"`offset + returned < total`"*,
 * deliberately not `total > returned`.
 *
 * So this axis alone can say how much it is **not** showing without inventing anything — which is
 * why it is declared rather than left to the reader to infer, and why the arm is bounded by a page
 * rather than read to exhaustion.
 */
export interface SeedAxis {
	shown: number;
	total: number;
	truncated: boolean;
}

/**
 * The entry read's own bound declaration.
 *
 * §7.1: the bound line is **chrome, not a warning** — present whether or not the view is partial,
 * *"so complete is something the reader is TOLD rather than something they infer from silence."*
 * Every other axis on this screen gets its numbers from the composition trace. The entry read runs
 * no composition, so it carries its own, and this is where they land.
 *
 * `inScope - eligible` is the count of resources deliberately **not drawn** for having no visible
 * connections. It is declared rather than dropped because
 * `legibility-is-never-bought-with-silent-omission` is the clause this goal sits under — on the
 * corpus that produced this design that difference is 1,077 of 3,574, and quietly omitting them is
 * a bigger silence than the 244-of-250 band it replaced.
 */
export interface OrientationAxis {
	drawn: number;
	/** How many cleared the connection floor — the denominator `drawn` is *of*. */
	eligible: number;
	/** Everything visible in the places asked about, connected or not. */
	inScope: number;
	truncated: boolean;
}

/**
 * What a traversal knows about its own bounds — and it is a short list.
 *
 * §7.1 forbids both available shortcuts: the line *"must not keep displaying the grounding query's
 * counts — on hop three those describe a screen the reader is no longer looking at"*, and it *"must
 * not disappear either: it is deliberately chrome, not a warning."* So the traversal carries its own.
 *
 * **There is no `drawn of eligible` ratio here and none may be manufactured.** `traversal_slice`
 * returns everything reached at the requested depth — it withheld nothing, so there is no
 * denominator, and inventing one would describe a selection that never happened.
 * {@link OrientationAxis} states a ratio because the entry read genuinely ranks and cuts; this read
 * does neither.
 *
 * @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §5
 */
export interface TraversedAxis {
	/** Marks on this screen. Not `of` anything — see above. */
	drawn: number;
	/**
	 * How many of them the reader hopped from.
	 *
	 * Zero is a real and reportable state, not a missing measurement: the service returns a seed
	 * *"that reached nothing"*, so a seed missing from the response was not visible to this reader.
	 * That is worth telling them, because it is also why nothing on screen carries a ring.
	 */
	from: number;
	/** Hops walked — the `depth` the read actually ran at, after the service's `1..=3` clamp. */
	depth: number;
}

export interface BoundDeclaration {
	/**
	 * The only axis the client enumerated both halves of itself.
	 *
	 * **`null` on a view with no place scope at all.** §10.3 rules that a traversal walks *"without a
	 * question locking you in"* — `traversal_slice` runs `graph_induced_edges` over the reader's
	 * whole visible corpus — so a place count there would not be a smaller number, it would be a
	 * **false claim of confinement**: `12 of 12 places` says the walk was scoped to twelve places
	 * when it was scoped to everything.
	 */
	places: { asked: number; available: number } | null;
	/**
	 * `null` when this read has no grouping axis at all, as against
	 * `{ applicable: false }` — which says the axis exists and did not apply.
	 *
	 * The two are not the same claim and must not render alike, which is the same distinction
	 * {@link GroupingsAxis} already draws one level down between *ran no funnel* and *ran one and
	 * disclosed no width*.
	 */
	groupings: GroupingsAxis | null;
	/** The list-read arm. `null` when the entry has no seed rows of its own — a question entry. */
	inYourPlaces: SeedAxis | null;
	/**
	 * Rows from the arm bounded in groupings and UNBOUNDED in rows. `null` when the entry has no
	 * such arm at all, which is absence rather than zero and renders as absence.
	 *
	 * It carries **no remainder claim**, and that is not an omission: its act reports
	 * `Extent::Indeterminate` unconditionally (`query_read.rs:737-743`) — *"a region funnel produces
	 * its candidate set rather than selecting from one, so there is no remainder to report."*
	 */
	fromYourPlaces: number | null;
	/** The walked arm — the only one whose `Extent` can ever say complete or partial. */
	followedOn: { rows: number; extent: Extent } | null;
	/**
	 * The entry read's axis. `null` on every view that ran a composition — which is all of them
	 * except the door for a reader who asked nothing.
	 */
	orientation: OrientationAxis | null;
	/** The traversal read's axis. `null` on every view that did not walk. */
	traversed: TraversedAxis | null;
}

const rowsOf = (stage: StageResult | undefined): number => stage?.produced?.hits?.length ?? 0;

/**
 * Read the applied funnel width off the response.
 *
 * **`terms_applied`, never the plan's own `terms`.** The contract's word for this field is *"the
 * APPLIED value of every admitted term: the page this stage actually RAN with, clamped to the act's
 * published ceiling and defaulted where the caller named nothing"*, and its other half is that
 * *"reporting the request back would make `terms_applied` an echo rather than a disclosure."* A
 * surface that rendered what it asked for would reintroduce exactly that echo one layer up.
 */
const appliedGroupings = (response: QueryResponse, stages: string[]): number | null => {
	for (const name of stages) {
		const applied = response.returned?.[name]?.terms_applied?.regions;
		if (applied !== undefined && applied !== null) return Number(applied);
	}
	return null;
};

export function declareBounds(
	response: QueryResponse,
	plan: GraphPlan,
	seeds: SeedAxis | null = null,
): BoundDeclaration {
	const hasFunnelArm = plan.surveyStages.length > 0;
	const walk = response.returned?.[plan.walkStage];

	return {
		places: { asked: plan.anchorsAsked.length, available: plan.anchorsAvailable },
		inYourPlaces: seeds,
		groupings: hasFunnelArm
			? { applicable: true, applied: appliedGroupings(response, plan.surveyStages) }
			: { applicable: false },
		fromYourPlaces: hasFunnelArm
			? plan.surveyStages.reduce((n, s) => n + rowsOf(response.returned?.[s]), 0)
			: null,
		followedOn: walk ? { rows: rowsOf(walk), extent: walk.extent } : null,
		orientation: null,
		traversed: null,
	};
}

/**
 * Declare the entry read's bounds — the composition-free path.
 *
 * Every axis a composition would have filled is **absent rather than zero**, and the difference
 * matters: this view ran no funnel, so `groupings` is `applicable: false` (the third state, not a
 * width of none), and it followed nothing on, so `followedOn` is null rather than `0 rows`.
 * Reporting them as zeros would describe a composition that returned nothing, which is a different
 * claim about the reader's corpus than one that was never run.
 */
export function declareEntryBounds(
	bounds: OrientationAxis,
	places: { asked: number; available: number },
): BoundDeclaration {
	return {
		places,
		inYourPlaces: null,
		groupings: { applicable: false },
		fromYourPlaces: null,
		followedOn: null,
		orientation: bounds,
		traversed: null,
	};
}

/**
 * Declare a traversal's bounds — the read that withheld nothing and cannot see past its own depth.
 *
 * Every composition axis is absent, and **`places` and `groupings` are absent here where the entry
 * read merely marks them inapplicable.** That is not a tidier spelling of the same thing. The entry
 * read *is* confined to the places asked (`readEntry` takes the anchor ids), so `12 of 12 places`
 * describes its scope truthfully. A traversal has no place scope at all — §10.3 rules the walk goes
 * on *"without a question locking you in"* — so the same phrase would assert a confinement that is
 * not there.
 *
 * **No ratio, and this is the rule it inherits verbatim.** `declareEntryBounds` records that absent
 * axes are *"absent rather than zero… reporting them as zeros would describe a composition that
 * returned nothing, which is a different claim."* The traversal's version of that error would be a
 * `drawn of eligible`: it returned everything it reached, so a denominator could only be invented.
 *
 * **It cannot know whether a deeper hop finds more, and does not say.** What it can say is that it
 * is complete *at this depth*, which is a narrower and true claim — and the one that keeps
 * `legibility-is-never-bought-with-silent-omission` covered on a screen the composition trace no
 * longer reaches.
 */
export function declareTraversalBounds(bounds: TraversedAxis): BoundDeclaration {
	return {
		places: null,
		groupings: null,
		inYourPlaces: null,
		fromYourPlaces: null,
		followedOn: null,
		orientation: null,
		traversed: bounds,
	};
}

/**
 * The reader's words for an `Extent`.
 *
 * The server's `indeterminate` reason is deliberately **not** echoed. It is honest prose written for
 * a caller, and it names the mechanism — *"a region funnel…"* — so passing it through would leak the
 * internal vocabulary via a field this surface does not control. The reader is told the same fact in
 * their own terms instead.
 */
const extentPhrase = (extent: Extent): string => {
	switch (extent.extent) {
		case 'complete':
			return 'complete';
		case 'partial':
			return 'more exist';
		default:
			return 'completeness not reported';
	}
};

/**
 * **"per place", not "asked"** `[decided — 2026-08-20, Pete]`.
 *
 * This axis and the readout's count are different aggregations of one word. `terms_applied[regions]`
 * is the funnel width **one** survey ran at; the readout counts the distinct groupings the whole
 * answer drew on, across every anchor. Measured against production they read `3` and `15` — both
 * true, 5× apart, and side by side on one screen under the same noun. Saying *per place* is what
 * lets a reader reconcile them instead of concluding they have misunderstood something.
 */
const groupingsPhrase = (axis: GroupingsAxis): string => {
	if (!axis.applicable) return 'groupings not applicable';
	return axis.applied === null ? 'groupings not reported' : `${axis.applied} groupings per place`;
};

/**
 * Render the line. Present whether the view is complete or partial, so **complete is something the
 * reader is told** rather than something they infer from silence — deliberately not the cheaper
 * "show a marker when something was dropped", under which the absence of a marker becomes the signal
 * and a bug that suppresses it is invisible.
 *
 * The two returned arms are declared **separately and never aggregated**, so no arm's truthfulness is
 * diluted by another's: the unbounded arm carries a count and makes no remainder claim, the walked
 * arm carries its own `Extent`.
 */
export function renderBoundLine(d: BoundDeclaration): string {
	const parts: string[] = [];

	if (d.orientation) {
		// Two numbers, not one, and never summed into a single "showing N". `drawn of eligible` is
		// what the reader is looking at; the unconnected remainder is a different fact about their
		// corpus, and collapsing them would hide exactly the omission this axis exists to declare.
		parts.push(`${d.orientation.drawn} of ${d.orientation.eligible} connected`);
		const unconnected = d.orientation.inScope - d.orientation.eligible;
		if (unconnected > 0) parts.push(`${unconnected} unconnected not drawn`);
	}

	if (d.inYourPlaces) {
		// The one arm that knows its own denominator, so it is the one arm that states one. Its
		// `truncated` is not rendered separately: `shown of total` already says it, and a second
		// phrase saying the same thing is how two halves of one fact start disagreeing.
		// The place phrase is dropped, not defaulted, when this read has no place axis: "across your
		// places" on a view with no place scope would be the same false confinement claim the axis
		// itself is null to avoid. The count keeps its denominator either way.
		const where =
			d.places === null ? null : d.places.asked === 1 ? 'in this place' : 'across your places';
		parts.push(`${d.inYourPlaces.shown} of ${d.inYourPlaces.total}${where ? ` ${where}` : ''}`);
	}
	if (d.traversed) {
		// No `of`. This read returned everything it reached, so there is no denominator to state and
		// none may be invented — see `declareTraversalBounds`.
		parts.push(`${d.traversed.drawn} ${d.traversed.drawn === 1 ? 'mark' : 'marks'}`);

		// Zero is reported rather than skipped, and it is the more useful case: it is why nothing on
		// screen carries a ring. Silence here would leave the reader to work that out.
		parts.push(
			d.traversed.from === 0
				? 'nothing you can see to have hopped from'
				: `${d.traversed.from} you hopped from`,
		);

		// Two clauses that do not fight: complete AT THIS DEPTH is something the read genuinely
		// knows, and what lies past it is something it genuinely does not. Collapsing them into one
		// `Extent` would have to pick a single word for two different states of knowledge.
		parts.push(
			`complete within ${d.traversed.depth} ${d.traversed.depth === 1 ? 'hop' : 'hops'}`,
			'deeper not reported',
		);
	}

	if (d.fromYourPlaces !== null) parts.push(`${d.fromYourPlaces} from your places`);
	if (d.followedOn) {
		parts.push(`${d.followedOn.rows} followed on`, extentPhrase(d.followedOn.extent));
	}
	// Both axes are pushed only when the read HAS them. A read with no grouping axis and no place
	// scope says neither, rather than saying `not applicable` twice about things it never had.
	if (d.groupings) parts.push(groupingsPhrase(d.groupings));
	if (d.places) {
		parts.push(
			`${d.places.asked} of ${d.places.available} ${d.places.available === 1 ? 'place' : 'places'}`,
		);
	}

	return `Showing ${parts.join(' · ')}`;
}
