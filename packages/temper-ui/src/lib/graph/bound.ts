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

export interface BoundDeclaration {
	/** The only axis the client enumerated both halves of itself. */
	places: { asked: number; available: number };
	groupings: GroupingsAxis;
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

	if (d.inYourPlaces) {
		// The one arm that knows its own denominator, so it is the one arm that states one. Its
		// `truncated` is not rendered separately: `shown of total` already says it, and a second
		// phrase saying the same thing is how two halves of one fact start disagreeing.
		const where = d.places.asked === 1 ? 'in this place' : 'across your places';
		parts.push(`${d.inYourPlaces.shown} of ${d.inYourPlaces.total} ${where}`);
	}
	if (d.fromYourPlaces !== null) parts.push(`${d.fromYourPlaces} from your places`);
	if (d.followedOn) {
		parts.push(`${d.followedOn.rows} followed on`, extentPhrase(d.followedOn.extent));
	}
	parts.push(groupingsPhrase(d.groupings));
	parts.push(
		`${d.places.asked} of ${d.places.available} ${d.places.available === 1 ? 'place' : 'places'}`,
	);

	return `Showing ${parts.join(' · ')}`;
}
