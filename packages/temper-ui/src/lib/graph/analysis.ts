import type {
	CogmapRegionMetricsRow,
	CogmapRegionRow,
	CogmapStaleness,
	ShapeEmptiness,
} from '$lib/types/generated/cognitive_maps';
import { relativeTime } from './relativeTime';

/**
 * The analysis surface — the machine's own measurements of one place, declared as such.
 *
 * **This is the receiver.** `RegionHoverCard.svelte:17-19` (at `87ccd211`, the last commit before Beat D deleted it) rendered
 * `memberCount` · `salience` · `coherence` on the navigational canvas; the successor takes that off the reader's path, and the
 * goal's `displaced-structure-remains-reachable` clause is only sound if the payload is rehomed
 * *somewhere that declares itself as analysis rather than as the reader's material*. That is this
 * module and the route that renders it, and nothing else on this surface.
 *
 * **Why this may print numbers when `readout.ts` may not.** `readout.ts` keeps `salience` off its
 * type deliberately — *"the number is never presented to the reader as a score"* — and that rule is
 * scoped to the **navigational** readout, where a figure would be an unexplained machine opinion
 * sitting beside the reader's own work. Here the whole page says it is the machine's measurements,
 * so the honest move is the opposite one: print exactly what the substrate holds and refuse to
 * normalise it. The two files say the same thing from different sides, and both say so out loud.
 *
 * **Nothing here is ever drawn as a percentage, a bar, a ratio or a 0–100 scale**, because none of
 * these quantities is bounded and their scales differ per place. Measured against the deployed
 * substrate on 2026-08-20:
 *
 * ```
 *                      cogmap (406 regions)    context @me/temper (501)
 * centrality           0 → 2342.2              0 → 276
 * reference_standing   0 → 96                  0 → 9
 * internal_tension     0 → 4.7                 0 → 0      ← identically zero, all 501
 * content_cohesion     0.879 → 1.000           0.872 → 1.000
 * telos_alignment      0.593 → 1.000           0.679 → 0.984
 * salience             median 0.95, max 497.65 median 0.55, max 69.54
 * ```
 *
 * A figure that *looks* normalised would settle an open ruling silently, which is the one outcome
 * worse than showing nothing. So a raw figure always appears beside this place's own measured
 * range, and a metric that turns out to be constant is **said** rather than ranked — an ordering
 * over 501 identical zeroes manufactures a rank that does not exist.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §4 (Beat C)
 */

/** The quantities this surface reports. Two come from the surface tier, five from the analytics tier. */
export type MetricKey =
	| 'member_count'
	| 'salience'
	| 'centrality'
	| 'content_cohesion'
	| 'internal_tension'
	| 'reference_standing'
	| 'telos_alignment';

/**
 * One reported quantity: plain words first, the substrate's own field beside it.
 *
 * `no-internal-vocabulary-is-load-bearing` is paid by `label` and `gloss` — a reader who has never
 * heard of a telos can read every row. `field` is shown too rather than hidden, because a surface
 * that declares itself as analysis and then renames the things it is analysing is harder to check,
 * not easier to read. The clause forbids *requiring* the internal word, not mentioning it.
 */
export interface MetricSpec {
	key: MetricKey;
	/** Plain words, and what leads. */
	label: string;
	/** The substrate's own column name, shown but never load-bearing. */
	field: string;
	/** One line saying what it measures, without assuming the reader knows how anything is derived. */
	gloss: string;
	/**
	 * What this quantity tells a reader, **and what it does not** — one line, in their words.
	 *
	 * `[from the reader session, 2026-08-21]` *"the boxes about salience, centrality, cohesion vs
	 * tension etc are cool but if I wasn't a data nerd I wouldn't know what this is trying to tell
	 * me … the boxes describing the counts et al might need to be a bit more 'why this matters' or
	 * 'what this tells you and what it doesnt' inviting."*
	 *
	 * The second half is the load-bearing half. A gloss that only says what a number *is* invites a
	 * reader to supply their own meaning for it, and the meanings they supply are usually stronger
	 * than the number can carry — `salience` reads as *importance*, `cohesion` as *correctness*.
	 * Naming the limit is how the surface avoids implying a judgement it never made.
	 */
	reading: string;
}

export const METRICS: MetricSpec[] = [
	{
		key: 'member_count',
		label: 'Resources in it',
		field: 'member_count',
		gloss:
			'How many of your resources this grouping holds — counted over the ones you can read, so two readers can honestly see different numbers.',
		reading:
			'Tells you how much material a grouping holds. It does not tell you the grouping is any good — a large one can simply be where everything unsorted ended up.',
	},
	{
		key: 'salience',
		label: 'How strongly this place ranks it',
		field: 'salience',
		gloss:
			'A blend the system computes to decide what to show first. It is unbounded and it is not a fraction — on this data it runs past 490.',
		reading:
			'Tells you what this place will put in front of you first. It does not tell you what matters — it is the order the system uses, not a judgement about your work.',
	},
	{
		key: 'centrality',
		label: 'Pull from its own members',
		field: 'centrality',
		gloss:
			'How much declared affinity the members hold for each other, scaled by how many there are.',
		reading:
			'Tells you how hard the members pull on each other. It does not tell you they belong together — only that they are linked.',
	},
	{
		key: 'content_cohesion',
		label: 'How alike the members are',
		field: 'content_cohesion',
		gloss:
			'The average closeness of each member to the middle of the group. Nearer 1 is more alike.',
		reading:
			'Tells you how alike the members read. It does not tell you they are about the same thing; similar wording is not a shared subject.',
	},
	{
		key: 'internal_tension',
		label: 'Disagreement among the members',
		field: 'internal_tension',
		gloss:
			'The combined weight of links between members that contradict each other. Tension binds a grouping together; it never splits one.',
		reading:
			'Tells you where members contradict each other. It does not mean the grouping is broken — tension is what binds one together, and it never splits one.',
	},
	{
		key: 'reference_standing',
		label: 'How often it has been leaned on',
		field: 'reference_standing',
		gloss: 'How many times the members have been reinforced by later work.',
		reading:
			'Tells you how often later work leaned on these. It does not tell you they were right — only that they were used.',
	},
	{
		key: 'telos_alignment',
		label: 'Closeness to what this map is for',
		field: 'telos_alignment',
		gloss:
			'How near this grouping sits to the charter the map was built around. Absent when the charter has nothing to compare against.',
		reading:
			'Tells you how near this sits to what the map was built for. A low figure does not mean off-topic; the charter may simply not speak to it.',
	},
];

/** One grouping, joined across the surface tier and the analytics tier. */
export interface AnalysedRegion {
	regionId: string;
	lensId: string;
	/** Agent-authored, and the only text on this surface that is not the machine's own arithmetic. */
	label: string | null;
	/** `null` is **not computed**, and must never render as `0`. */
	values: Record<MetricKey, number | null>;
}

export interface AnalysedShape {
	regions: AnalysedRegion[];
	/**
	 * `false` when the region-metrics read did not answer.
	 *
	 * The five analytics-tier scalars are then **unknown**, not absent — captioning 501 regions
	 * *"not computed"* would be a claim about the substrate made on evidence the surface does not
	 * have. Same posture as `RegionLookup.complete`, one read over.
	 */
	metricsAvailable: boolean;
}

/** The five scalars, absent. Used for a row the metrics read did not cover and for a read that failed. */
const NO_SCALARS = {
	centrality: null,
	content_cohesion: null,
	internal_tension: null,
	reference_standing: null,
	telos_alignment: null,
} as const;

/**
 * Join the two reads into one row per grouping.
 *
 * **Keyed on the region AND its lens.** Measured on live data every anchor has exactly one lens, so
 * `region_id` alone is unique today — which is precisely the condition the survey act warns is
 * *"invisible today only because contexts have exactly one lens."* Keying on the region alone would
 * pair a grouping with another lens's numbers the day that stops holding, silently and with no test
 * able to see it.
 *
 * **Shape drives the row set.** It is the tier that carries the reader-facing label and the member
 * count; a metrics row with no shape row is a grouping the surface tier does not publish, and
 * inventing a row for it would put a thing on screen that the place itself does not list.
 */
export function analyseShape(
	shape: CogmapRegionRow[],
	metrics: CogmapRegionMetricsRow[] | null,
): AnalysedShape {
	const key = (regionId: string, lensId: string) => `${regionId} ${lensId}`;
	const byKey = new Map((metrics ?? []).map((m) => [key(m.region_id, m.lens_id), m]));

	return {
		metricsAvailable: metrics !== null,
		regions: shape.map((s) => {
			const m = byKey.get(key(s.region_id, s.lens_id));
			return {
				regionId: s.region_id,
				lensId: s.lens_id,
				label: s.label,
				values: {
					// The surface tier's own two. They survive a metrics read that never answered,
					// because they never came from it.
					member_count: s.member_count,
					salience: s.salience,
					...(m
						? {
								centrality: m.centrality,
								content_cohesion: m.content_cohesion,
								internal_tension: m.internal_tension,
								reference_standing: m.reference_standing,
								telos_alignment: m.telos_alignment,
							}
						: NO_SCALARS),
				},
			};
		}),
	};
}

/**
 * What one metric actually does across this place.
 *
 * `min`/`max` are what let a raw figure be situated without normalising it, and `constant` is the
 * finding a ranked list would have destroyed.
 */
export interface Distribution {
	/** How many groupings carry a value. */
	n: number;
	/** How many do not — stated, never left to look like zeroes. */
	nulls: number;
	min: number | null;
	median: number | null;
	max: number | null;
	/** Every value identical, over more than one grouping. */
	constant: boolean;
}

export function distributionOf(regions: AnalysedRegion[], k: MetricKey): Distribution {
	const vs = regions
		.map((r) => r.values[k])
		.filter((v): v is number => v !== null)
		.sort((a, b) => a - b);

	if (vs.length === 0) {
		return { n: 0, nulls: regions.length, min: null, median: null, max: null, constant: false };
	}

	return {
		n: vs.length,
		nulls: regions.length - vs.length,
		min: vs[0],
		median: vs[Math.floor(vs.length / 2)],
		max: vs[vs.length - 1],
		// One value is a value, not a distribution — claiming "every grouping measures X" of a
		// single grouping would be a statement about a population of one.
		constant: vs.length > 1 && vs[0] === vs[vs.length - 1],
	};
}

/**
 * A raw figure, exactly as the substrate holds it.
 *
 * The only transformation is a **correction**, not a rounding down of information: IEEE754 hands
 * back `2342.200000000001` and `303.99999999999994` for quantities that are 2342.2 and 304, and
 * printing that noise would imply a precision the computation does not have.
 *
 * `null` is a dash. It is never a zero, and the distinction is load-bearing on this surface:
 * `internal_tension` genuinely IS zero for every grouping in a context, and a reader has to be able
 * to tell that from a value nobody computed.
 */
export function formatValue(v: number | null): string {
	if (v === null) return '—';
	return String(Number(v.toFixed(4)));
}

/** This place's own measured span, named as this place's and no one else's. */
export function describeRange(d: Distribution): string {
	if (d.min === null || d.max === null) return 'no values here';
	return `here: ${formatValue(d.min)} – ${formatValue(d.max)}`;
}

/**
 * A metric that does not vary, said rather than ordered.
 *
 * Measured: `internal_tension` is identically `0` across all 501 groupings of `@me/temper`. Sorting
 * that column would produce an order, and every reader of that order would be reading noise.
 */
export function describeConstant(d: Distribution): string {
	return `Every grouping here measures ${formatValue(d.min)}.`;
}

/** How many groupings the machine has no figure for — stated, so a gap never reads as a zero. */
export function describeNulls(d: Distribution): string | null {
	if (d.nulls === 0) return null;
	return `${d.nulls} of ${d.n + d.nulls} have no value for this.`;
}

/**
 * When this place's shape was last worked out, and whether the work has moved on since.
 *
 * **Staleness is legible, never blocking** — the substrate's own words. So none of these sentences
 * may read as an error, and the never-materialized case is a different statement from a stale one:
 * a shape that has never been computed is not an out-of-date shape.
 */
export function describeStaleness(s: CogmapStaleness): string {
	if (s.materialized_at === null) return 'This shape has never been worked out.';

	const when = relativeTime(s.materialized_at);
	if (!s.is_stale) return `This shape was worked out ${when}, and nothing has changed since.`;

	const touched = s.latest_touch ? relativeTime(s.latest_touch) : 'since';
	return `This shape was worked out ${when}. Your work here has changed since then — ${touched} — so it is being read as it stood, not as it is now.`;
}

/**
 * Which quantities earn a column, and which collapse to one sentence.
 *
 * A column of 501 identical values is not a measurement a reader can use — it is the same fact
 * repeated 501 times, and sorting it would produce an order that is pure noise. Measured:
 * `internal_tension` is identically `0` for every grouping of `@me/temper`. So a constant metric,
 * and one nothing has computed at all, are **stated in the legend and given no column**.
 *
 * This is not a bound: nothing is withheld. The value is on screen, once, because once is how many
 * distinct values there are.
 */
export interface MetricReport {
	spec: MetricSpec;
	distribution: Distribution;
	/** False when the metric is constant across this place, or has no values here at all. */
	asColumn: boolean;
}

export function reportMetrics(regions: AnalysedRegion[]): MetricReport[] {
	return METRICS.map((spec) => {
		const distribution = distributionOf(regions, spec.key);
		return { spec, distribution, asColumn: distribution.n > 0 && !distribution.constant };
	});
}

/**
 * Why a context shows no charter and no regulation — declared, never faked as a peer field.
 *
 * **A context IS asked for staleness now**, and the clock it answers with renders beside this
 * sentence: `/api/contexts/{id}/analytics` returns three fields where the cogmap door returns five.
 * What it does not return is the other two, and those two are what this sentence is about. A
 * context has a `telos_centroid` and neither a charter resource nor a regulation set, so for those
 * there is nothing to return even in principle — and a null peer field would report *nothing
 * found* about two things that cannot exist.
 */
export const CONTEXT_HAS_NO_MAP_READOUT =
	'This is a context. A charter, and the concepts that regulate it, belong to a cognitive map — a context has neither, so there is nothing here to report rather than nothing found. What is measured below is its groupings.';

/** The map-level read was declined. A deny renders as unavailable, never as an error. */
export const MAP_READOUT_UNAVAILABLE =
	'The map-level picture is not available for this map just now.';

/**
 * The regulation set, said rather than counted to zero.
 *
 * Measured across all four readable maps on 2026-08-20: **every one returns `[]`**. So the empty
 * state is the routine case, not an edge case, and it has to read as a fact about the map rather
 * than as a failure of the page.
 */
export function describeRegulation(n: number): string {
	if (n === 0) return 'No concepts have been set to regulate this map.';
	return `${n} ${n === 1 ? 'concept regulates' : 'concepts regulate'} this map.`;
}

/**
 * Why an empty groupings list is empty, in the reader's own terms.
 *
 * **This is the sentence the whole task turns on.** An empty list is four different situations, and
 * until the envelope crossed the door this page spelled all four *"This place has no groupings
 * yet."* — a sentence whose *yet* asserts `never_clustered`. For a reader under `nothing_visible`
 * it claimed nothing had been built when the place had in fact been clustered; under
 * `unreadable_or_absent` it described a place they cannot read at all as though it were their own
 * and merely young. The same claim-a-cause-you-cannot-know defect was fixed at the CLI door in
 * `16a9e357`; this was its last unfixed instance, and the one door where the reader is a person.
 *
 * **`nothing_visible` deliberately does not separate its two causes, and this wording must not
 * either.** It is reached both when the anchor formed no regions at all and when it formed regions
 * holding nothing this reader can see. Splitting them would tell a caller how many resources they
 * cannot read, which is precisely what the member gate forbids
 * (`migrations/20260713000050_region_visible_member_count.sql:137`, *"a caller is never told how
 * many resources they cannot read"*). So the sentence names **both** possibilities and then says
 * outright that the reader must not read it as missing access — because a reader told only the
 * second half would conclude exactly that, and a reader told only the first would be misinformed.
 * The reasoning is recorded at the SQL arm and in
 * `internal/superpowers/specs/2026-08-23-anchor-shape-envelope-design.md`. **Do not add a fifth
 * case here.** The ambiguity reads like a gap and is load-bearing.
 *
 * **Each sentence names a next move, because the four causes differ in what to DO, not in tone.**
 * `never_clustered` is the only one with an action, and it names the command, the permission it
 * needs, and who to ask without it — the CLI and MCP doors already say *run `context materialize`*,
 * and withholding it from the one reader who cannot look it up in a Rust signature inverts the whole
 * point of this work. There is no in-app way to materialize, so the sentence says the exit is
 * outside the page rather than leaving the reader to wait for something that will never happen.
 *
 * **One verb for one event.** `describeStaleness` already spells this event *"worked out"*, and it
 * renders on the SAME page eight lines above this one for the same never-materialized anchor
 * (pinned by `analysis.captured.test.ts`'s `cogmap_never_materialized` fixture). An earlier draft
 * said *"groupings are built by a separate pass"* — a third noun and a third verb for one thing,
 * which reads to a non-expert as two separate things that have each not happened.
 *
 * **`unreadable_or_absent` is about the MEASUREMENTS, not the place's existence.** On this route the
 * `<h1>` has already rendered the place's title, from the reader's own anchor list — so a sentence
 * claiming the page cannot say whether it is there contradicts the heading directly above it. The
 * existence was never disclosed BY this arm (it came from the reader's own listing), so this is a
 * page contradicting itself rather than a leak; the wording is scoped to what this read could not
 * fetch.
 *
 * **`lens_narrowed` cannot arrive at this door today**, and is spelled anyway. `readAnchorAnalysis`
 * passes no `lens` — the lens is a clustering-time parameter — and with `p_lens IS NULL` the shape
 * function's row filter and its `population` count range over the same set, so arm 3 can never fire
 * (`migrations/20260823000010_anchor_shape_envelope.sql:121-122` and `:132`). It is written rather than thrown
 * to a default so the match stays exhaustive over {@link ShapeEmptiness}: the day a caller does
 * pass a lens, `tsc` will already have been satisfied and the reader will already have a sentence.
 */
/** The place the sentence is about, structurally — avoids importing `NamedPlace` from `entry.ts`. */
export type PlaceRef = { kind: 'context' | 'cogmap'; ref: string } | null;

/**
 * Refs safe to render inside a command line a reader will paste.
 *
 * **Rendered as a guard rather than asserted as an invariant, because it is not one.** A cogmap's
 * `ref` is `m.id`, a bare uuid, and is safe by construction. A context's is `${owner_ref}/${slug}`,
 * and no production gate on that slug could be found: `validate_slug` guards `CreateResource` only,
 * every `INSERT INTO kb_contexts` in the tree is in a test, and the column carries no CHECK. So
 * "context slugs are lowercase-and-hyphens" is how they all look, not something enforced where this
 * code can see it.
 *
 * A ref that fails this drops the ARGUMENT, not the command — the reader gets a line that is correct
 * and needs one more word, instead of one that looks exact and is wrong. That direction matters more
 * here than usual: this whole surface exists because it used to state things it could not know.
 */
const PASTE_SAFE_REF =
	/^(?:[@+][a-z0-9][a-z0-9-]*\/[a-z0-9][a-z0-9-]*|[a-z0-9-]*[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})$/;

/**
 * The command this reader would run, as a bare command line — or `null` when there is none to give.
 *
 * **Returned separately from the sentence so the page can render it as a `<code>`.** An earlier
 * draft interpolated it into the prose wrapped in backticks, which read correctly in the source and
 * rendered as *literal backtick characters* in an italic paragraph: the one string on the page meant
 * to be copied verbatim, set in the one treatment that makes a command hard to read, with stray
 * punctuation glued to both ends. Nothing caught it — every assertion used `toContain`, and the
 * backticks were inside the expected substring. It took looking at the page.
 */
export const materializeCommandFor = (
	emptiness: ShapeEmptiness | null,
	place: PlaceRef,
): string | null => {
	// Only one arm has an action, and only when the page knows which kind of place it is on.
	if (emptiness !== 'never_clustered' || !place) return null;

	const cmd = place.kind === 'cogmap' ? 'temper cogmap materialize' : 'temper context materialize';
	return PASTE_SAFE_REF.test(place.ref) ? `${cmd} ${place.ref}` : cmd;
};

const describeEmptyShape = (emptiness: ShapeEmptiness | null, place: PlaceRef): string => {
	switch (emptiness) {
		case 'never_clustered':
			// The command is NOT in this sentence; the page renders it beneath, as a `<code>`. What
			// stays here is the lead-in, and it has to read correctly whether or not one follows —
			// `place: null` yields no command, and this still has to be a whole sentence there.
			return place
				? 'This place has never been grouped — its groupings have not been worked out yet. Nothing here is broken. If you do not have write access to the place, ask whoever does; anyone who has it can work them out by running:'
				: 'This place has never been grouped — its groupings have not been worked out yet. Nothing here is broken. Anyone with write access to the place can work them out with the materialize command; if you do not have write access, ask whoever does.';
		case 'nothing_visible':
			return 'This place has been grouped, and nothing came back that you can read. It may have formed no groupings at all, or the groupings it formed may hold only work that is not yours to see. This page deliberately cannot tell you which, so it is not evidence that you are missing access — if you expected to find work here, ask whoever runs this place.';
		case 'lens_narrowed':
			return 'This place has groupings, and the view you have selected excludes every one of them. Widening the view — or clearing it altogether — will bring them back.';
		case 'unreadable_or_absent':
			return 'The measurements for this place could not be read. Either they are not readable by you or it is not there any more — this page deliberately cannot tell you which, because saying which would answer the other. If you followed a link here, check it with whoever sent it.';
		case null:
		// The read said the set was non-empty while handing back no rows. The server does not
		// produce this — `emptiness` is non-NULL exactly when the row set is empty — so rather
		// than invent a cause for a state that should not exist, say only what is known.
		//
		// `default` shares this arm and is NOT dead code, however exhaustive the union looks.
		// `undefined` reaches here from an API build predating `20260823000010`, which sends no
		// `emptiness` key at all (the field carries no `skip_serializing_if`, so this needs version
		// skew rather than an ordinary response). Without it the switch falls off the end, the
		// function returns `undefined` from a `: string` signature, and the page renders the literal
		// word "undefined" under "How its work has been grouped" — failing ugly on precisely the
		// door this work exists to make honest.
		default:
			return 'No groupings came back, and the read did not say why.';
	}
};

/**
 * How many groupings this place publishes, and in whose order they are shown — or, at zero, **why
 * there are none**.
 *
 * `emptiness` is required rather than optional on purpose. Making it optional would leave the old
 * one-argument call compiling and still asserting a cause it cannot know; requiring it makes `tsc`
 * name every site that has to supply one. See {@link describeEmptyShape}.
 */
export function describeGroupingCount(
	n: number,
	emptiness: ShapeEmptiness | null,
	place: PlaceRef,
): string {
	if (n === 0) return describeEmptyShape(emptiness, place);
	return `${n} ${n === 1 ? 'grouping' : 'groupings'}, in the order this place itself ranks them.`;
}

/** The deeper measurements did not answer. Unknown, and it must not read as absent or as zero. */
export const METRICS_UNAVAILABLE =
	'The deeper measurements could not be read just now, so they are shown as unknown rather than as nothing.';

/**
 * Where a grouping's mark sits on a quantity's own axis, and whether that axis is compressed.
 *
 * **Why this is not simply `(v - min) / (max - min)`.** Measured on the captured context: a linear
 * axis puts **94%** of `salience` marks, **94%** of `centrality` and **97%** of `reference_standing`
 * inside the first tenth of the width — a smear against the left edge with one dot far right. That
 * is a correct picture of the numbers and a useless one to look at, which is the same complaint the
 * table drew, reproduced in a new medium.
 *
 * So a heavily right-skewed quantity is spaced by `log1p`. The rule is **derived from the data, not
 * chosen per metric**: if the median sits below a fifth of the span, a linear axis would put more
 * than half the marks in a fifth of the width. That matters because these quantities are unbounded
 * and the corpus keeps moving — a hand-picked list of "the log ones" would be right today and wrong
 * after a re-clustering, silently.
 *
 * **The axis labels stay the real figures either way**, and a compressed axis says so — see
 * {@link describeAxis}. Distance on a compressed axis is not difference, and a reader who is not
 * told that will read it as if it were.
 */
export interface Axis {
	min: number;
	median: number;
	max: number;
	/** `log1p` spacing, because a linear axis would hide most of the marks in a tenth of the width. */
	compressed: boolean;
}

/** The fraction of the span below which a median makes a linear axis unreadable. Measured, not tuned. */
const SKEW_LIMIT = 0.2;

/** `null` when there is nothing to plot: no values, or no spread to plot them against. */
export function axisFor(d: Distribution): Axis | null {
	if (d.min === null || d.max === null || d.median === null || d.max === d.min) return null;
	return {
		min: d.min,
		median: d.median,
		max: d.max,
		compressed: (d.median - d.min) / (d.max - d.min) < SKEW_LIMIT,
	};
}

/** A value's place on its axis, `0`–`1`. */
export function positionOn(v: number, a: Axis): number {
	const span = a.max - a.min;
	if (span === 0) return 0;
	return a.compressed ? Math.log1p(v - a.min) / Math.log1p(span) : (v - a.min) / span;
}

/**
 * The sentence a compressed axis owes its reader.
 *
 * Without it the plot silently changes what distance means, which is the same class of error as
 * showing an unbounded quantity on a 0–100 scale: the picture would look calibrated and would not be.
 */
export function describeAxis(a: Axis): string | null {
	return a.compressed
		? 'Spacing is compressed — most groupings sit near the low end, so equal gaps here are not equal differences.'
		: null;
}

/**
 * *Most of them measure the same thing* — said, when a plot would show it as a smear.
 *
 * `describeConstant` covers the case where a quantity does not vary at all. This is the near case,
 * which no axis can rescue: measured on the captured context, **97% of groupings have a
 * `reference_standing` of 0**, so the marks pile at one point however they are spaced. A picture of
 * that is honest only if it is accompanied by the number.
 */
export function describeConcentration(regions: AnalysedRegion[], k: MetricKey): string | null {
	const vs = regions.map((r) => r.values[k]).filter((v): v is number => v !== null);
	if (vs.length === 0) return null;
	const counts = new Map<number, number>();
	for (const v of vs) counts.set(v, (counts.get(v) ?? 0) + 1);
	let mode = vs[0];
	let best = 0;
	for (const [v, n] of counts) if (n > best) [mode, best] = [v, n];
	// Below half, the pile is a feature of the distribution rather than the whole of it.
	if (best / vs.length < 0.5 || best === vs.length) return null;
	return `${best} of ${vs.length} measure ${formatValue(mode)}.`;
}
