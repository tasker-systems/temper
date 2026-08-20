import type {
	ActInvocation,
	CombineNode,
	Composition,
	IdKind,
	StageInput,
	StageNode,
} from '$lib/types/generated/query';

/**
 * The composition builder for the graph surface: `(anchors, question, seeds) → Composition`.
 *
 * Every shape here is the generated `query.ts` — the wire contract's own types, never a
 * hand-written mirror. Three plans, per the surface spec §2, and the degenerate case of the first:
 *
 * - **a question, N ≥ 2 anchors** — N × `survey` → `union` → `follow-from`; surveys and walk both returned
 * - **a question, exactly 1 anchor** — `survey` → `follow-from`, with NO combinator (see below)
 * - **a question, 0 anchors** — `find-about-anywhere` → `follow-from`
 * - **no question** — N × `find-resources-with` → (`union`) → `follow-from`; only the walk returned
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §2, §3
 */

/**
 * One place a reader can be asked about. Both kinds carry the same fields deliberately: an ordering
 * that read a field only one kind has could not span both, which is the reachability the surface
 * exists to provide.
 */
export interface Anchor {
	kind: 'context' | 'cogmap';
	/** The `kb_contexts` / `kb_cogmaps` row id. What the anchor `IdSet` actually carries. */
	id: string;
	/** `@owner/slug`, `+team/slug`, or a cogmap's decorated ref. The tie-break, and what a URL holds. */
	ref: string;
	resourceCount: number;
}

/**
 * How many anchors one composition may fan out over.
 *
 * `[decided — 2026-08-20, Pete]` against a measurement rather than a guess, as the spec's §7 required:
 * the heaviest real reader of the system (2,330 resources) holds **12** — 8 contexts and 4 cogmaps.
 * 24 is 2× that, so truncation does not fire for any real reader today and the ordering below is a
 * safety net rather than routine behaviour. A ceiling that fired routinely would make an ordinary act
 * — creating a context — silently change what the unaddressed door asks.
 *
 * There is no server-side ceiling to inherit: the validator caps no stage count, and union arity is
 * unbounded (`validate/shape.rs:150-175` refuses only `inputs.len() < 2`). This number is the
 * client's alone, which is exactly why the third bound axis has to declare it from the client's own
 * record — no `Extent` can ride on a truncation that happened before the composition existed.
 */
export const ANCHOR_CEILING = 24;

/**
 * The funnel width named on every `survey` stage.
 *
 * **Naming it is what makes the axis disclosable at all**, and it is not merely belt-and-braces:
 * `applied_terms` defaults ONLY `Limit`, and only from a published ceiling — `Regions` deliberately
 * does not (`registry.rs:689`), because defaulting it to the ceiling of 20 *"would widen every
 * unbounded survey sevenfold while claiming to describe the deployed system."* Its own test pins the
 * consequence: `applied_terms(&BTreeMap::new(), &survey).get(&Regions) == None`
 * (`registry.rs:1678-1682`).
 *
 * So a survey naming nothing runs at the fragment's own default of 3 and reports **nothing**, and the
 * bound line's second axis would have no source. 3 is that same fragment default, said out loud.
 */
export const REGIONS_PER_ANCHOR = 3;

/** `follow-from`'s published ceiling (`registry.rs:392`), named rather than left to the default. */
export const WALK_LIMIT = 50;

export interface GraphPlan {
	composition: Composition;
	/** The anchors that reached the plan, already ordered and truncated. */
	anchorsAsked: Anchor[];
	/** How many were available before the ceiling. The only bound axis with a true denominator. */
	anchorsAvailable: number;
	/** Named so the readout can find each arm without re-deriving the naming scheme. */
	surveyStages: string[];
	walkStage: string;
}

export type PlanOutcome = { ok: true; plan: GraphPlan } | { ok: false; reason: 'nothing-to-ask' };

export interface PlanInput {
	anchors: Anchor[];
	question: string | null;
	seeds: string[] | null;
	/**
	 * The denominator of the places axis, when it is not simply `anchors.length`.
	 *
	 * It differs in exactly one situation, and that situation is the reason this exists: the reader
	 * NAMED some places and one of them did not resolve — deleted, or no longer readable. The
	 * resolved set is what can be asked; the named count is what the reader believes they asked. A
	 * line reading *"1 of 2 places"* declares the drop; *"1 of 1"* would hide it, and the surface
	 * would answer a narrower question than the URL says while looking complete.
	 *
	 * This reveals nothing the refusal face protects. *"A place the reader cannot read is absent,
	 * never hinted at, and its absence is not distinguishable from its nonexistence"* — and a bare
	 * count cannot distinguish them either: one number covers deleted, never-existed and
	 * not-readable-by-you alike.
	 */
	available?: number;
}

/**
 * `resource_count` DESC, ties by `ref` ASC.
 *
 * The alternatives are unavailable rather than merely worse. Only `resource_count`, `name` and `ref`
 * span both anchor kinds — a cogmap summary carries **no timestamp at all**, so a recency ordering
 * is inexpressible without going kind-dependent, which is the very thing
 * `cross-kind-relationship-is-reachable` forbids. Ordering by `id` looks like a free recency (UUIDv7
 * leads with a timestamp) and is wrong: the seeded rows are sentinels, not v7, so they sort to one
 * extreme regardless of age. `ref` ASC alone would drop the 2,066-resource context before the empty
 * one, `+` preceding `@` in ASCII.
 *
 * So this drops the emptiest anchors first, and the tie-break keeps a URL reproducible.
 */
const byMaterialThenRef = (a: Anchor, b: Anchor): number =>
	b.resourceCount - a.resourceCount || a.ref.localeCompare(b.ref);

/** `IdKind` is an OPEN vocabulary, so the anchor kinds are named rather than cast. */
const idKindOf = (anchor: Anchor): IdKind => (anchor.kind === 'cogmap' ? 'cogmap' : 'context');

/** A caller-supplied anchor: exactly one id, bounding the stage. Zero ids is refused outright. */
const anchorBound = (anchor: Anchor): StageInput => ({
	from: 'caller',
	as: 'bound',
	ids: { kind: idKindOf(anchor), provenance: null, ids: [anchor.id] },
});

/** Every field of `ActInvocation`, so a stage is never missing one the wire requires. */
const act = (fields: {
	name: string;
	act: string;
	intention?: string | null;
	inputs?: StageInput[];
	terms?: ActInvocation['terms'];
}): ActInvocation => ({
	name: fields.name,
	act: fields.act,
	intention: fields.intention ? { query: fields.intention, embedding: null } : null,
	inputs: fields.inputs ?? [],
	terms: fields.terms ?? {},
	resource_filter: null,
	edge_filter: null,
	properties: [],
});

/**
 * Build the plan, or say there is nothing to ask.
 *
 * The 1-anchor case emits **no combinator**, and that is a contract constraint rather than an
 * optimisation: `validate/shape.rs:150` refuses `inputs.len() < 2` as `CombinatorArity` — *"One input
 * is not a combination."* A builder that always emitted a union would produce an invalid plan for the
 * single-anchor entry, which is §2.4, the most ordinary door of the three.
 */
export function buildGraphPlan({ anchors, question, seeds, available }: PlanInput): PlanOutcome {
	const anchorsAsked = [...anchors].sort(byMaterialThenRef).slice(0, ANCHOR_CEILING);

	// No place to ask about and no question to ask: there is no honest composition here, and an
	// empty one is a refusal (`NoStages`) rather than an empty answer. Say so in the type.
	if (anchorsAsked.length === 0 && !question) return { ok: false, reason: 'nothing-to-ask' };

	const stages: StageNode[] = [];
	const walkStage = 'w';

	// The stages whose ids feed the walk. Surveys when there is a question; selections when there is
	// not; a single unbounded find when there is no anchor at all.
	let upstream: string[];

	if (!question) {
		// §2.3 — a place with no question shows everything in it. `find-resources-with` carries no
		// ceiling whatsoever, so the seed set genuinely is every visible resource in the anchor.
		upstream = anchorsAsked.map((anchor, i) => {
			const name = `m${i + 1}`;
			stages.push(act({ name, act: 'find-resources-with', inputs: [anchorBound(anchor)] }));
			return name;
		});
	} else if (anchorsAsked.length === 0) {
		// The reader has organized nothing. `find-about-anywhere` declares `accepts_bounds: []` and
		// asker_holds *"a concept, no exact words; search everything I can see"* — the act that needs
		// no organization at all, which is `entry-does-not-presume-organization` at its limit.
		stages.push(act({ name: 'f', act: 'find-about-anywhere', intention: question }));
		upstream = ['f'];
	} else {
		// §2.1 / §2.4 — one survey per anchor. N stages sharing one question cost ONE embedding, not
		// N: `texts_to_embed` keys on distinct query TEXT (`query_read.rs:162-168`).
		upstream = anchorsAsked.map((anchor, i) => {
			const name = `s${i + 1}`;
			stages.push(
				act({
					name,
					act: 'survey',
					intention: question,
					inputs: [anchorBound(anchor)],
					terms: { regions: BigInt(REGIONS_PER_ANCHOR) },
				}),
			);
			return name;
		});
	}

	const surveyStages = question && anchorsAsked.length > 0 ? [...upstream] : [];

	// A union is a stage only when there is something to combine.
	let walkSeed: string;
	if (upstream.length > 1) {
		const union: CombineNode = { name: 'u', op: 'union', inputs: upstream };
		stages.push(union);
		walkSeed = 'u';
	} else {
		walkSeed = upstream[0];
	}

	// Explicit `from` seeds replace the upstream stage as what the walk grows from — the reader has
	// named where to walk from, and the pipe is no longer the answer to that.
	const walkInput: StageInput =
		seeds && seeds.length > 0
			? { from: 'caller', as: 'seed', ids: { kind: 'resource', provenance: null, ids: seeds } }
			: { from: 'upstream', as: 'seed', stage: walkSeed };

	stages.push(
		act({
			name: walkStage,
			act: 'follow-from',
			inputs: [walkInput],
			terms: { limit: BigInt(WALK_LIMIT) },
		}),
	);

	// A selection stage is refused in `returns` (`StageNotReturnable`) — it produces ids, not rows —
	// so the no-question entry returns the walk alone. The surveys ARE returnable and are returned
	// beside the walk, keyed separately: `QueryResponse.returned` is a map, and that keying is the
	// structural half of `no-cross-act-ranking`.
	const returns = [...surveyStages, walkStage].map((stage) => ({ stage, with: [] }));

	return {
		ok: true,
		plan: {
			composition: { outcome: { returns }, stages },
			anchorsAsked,
			anchorsAvailable: available ?? anchors.length,
			surveyStages,
			walkStage,
		},
	};
}
