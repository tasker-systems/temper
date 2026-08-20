import type { CogmapRegionRow } from '$lib/types/generated/cognitive_maps';
import type { QueryResponse, RegionDisclosure } from '$lib/types/generated/query';

/**
 * The *why-these* readout — machine reasoning about the answer, and never a thing in the graph.
 *
 * **This module is the only place derived structure exists on this surface.** That is
 * `no-derived-thing-poses-as-authored` as a structural property rather than a rule someone
 * remembers: regions reach the readout and nothing else, and a sibling module that reaches for one
 * fails a test rather than merely reading oddly.
 *
 * It is also where `no-internal-vocabulary-is-load-bearing` is paid. The readout must be readable
 * without the words *region*, *salience*, *wayfind* or *survey* — it says *"these came from N
 * groupings of your work"*, not *"3 regions by region_score"*.
 *
 * @see internal/superpowers/specs/2026-08-20-graph-successor-surface-design.md §3
 */

/**
 * What a grouping turned out to be when its id was looked up.
 *
 * Three states, and **they must never render alike**. The middle one is the whole reason this is a
 * union rather than a nullable name: a region id is *"not durable, and the instability is the
 * caller's to handle"* — `assert_region` mints a new id whenever a member set changes — so an id
 * disclosed in the trace can name nothing by the time the reader sees it. That must read as
 * *"this grouping has been re-derived"*, **never as an error and never as the reader's mistake**.
 *
 * `unchecked` exists so the surface cannot claim the middle state on missing evidence. If a
 * lookup did not complete, an unfound id proves nothing, and saying *re-derived* would be exactly
 * the manufactured certainty this surface refuses elsewhere.
 */
export type GroupingName =
	| { state: 'named'; label: string | null; memberCount: number }
	| { state: 're-derived' }
	| { state: 'unchecked' };

/**
 * One grouping of the reader's work that the answer drew on.
 *
 * **It carries no score, and the missing field is deliberate.** The blend the substrate orders
 * these by is an OPEN ruling — it spans `[-0.57, 1.05]`, so it can be negative and it exceeds one
 * — and the standing consequence for this surface is that the number is **never presented to the
 * reader as a score**. The readout may say which groupings the answer drew on and in what order;
 * it may not print the number or imply a calibrated scale. Keeping the field off the type is what
 * makes that unforgettable rather than merely documented.
 *
 * `salience` is off it for the same reason and one more: measured on `@me/temper` it runs to
 * **69.5**, so it is not a fraction either, and nothing on this surface may imply it is.
 * `memberCount` is safe where those are not — it is *"over the members **this caller can read**"*,
 * a count of the reader's own material rather than a machine's opinion of it.
 */
export interface Grouping {
	id: string;
	name: GroupingName;
}

/** What one stage was handed and what it could not use. Every stage, returned or not. */
export interface StageAccount {
	stage: string;
	act: string;
	handed: number;
	/**
	 * How many ids did not contribute, for ANY reason — deliberately one conflated number.
	 * Invisible, nonexistent and malformed are not separated: splitting them would make this a
	 * single-probe existence oracle. The reader learns that 28 of their 240 did not contribute,
	 * which is what legibility asks for. They do not learn why.
	 */
	unusable: number;
}

export interface Readout {
	groupings: Grouping[];
	stages: StageAccount[];
}

/**
 * The shape rows gathered for every anchor that disclosed anything, and whether that gathering was
 * complete.
 *
 * **One flat set, not one per anchor.** A region id is a uuid and is globally unique, so which
 * anchor's shape read produced the row does not matter to a lookup — and both anchor kinds answer
 * the same read (`anchor_shape_select`, `Vec<CogmapRegionRow>`, at
 * `/api/cognitive-maps/{id}/shape` and `/api/contexts/{id}/shape`), which is
 * `cross-kind-relationship-is-reachable` holding one layer down.
 *
 * `complete` is false when any of those reads did not answer. It degrades every unfound id to
 * `unchecked` rather than `re-derived` — conservative in the one direction that matters, since the
 * surface must never tell a reader their grouping is gone on evidence it does not have.
 */
export interface RegionLookup {
	rows: CogmapRegionRow[];
	complete: boolean;
}

/** Nothing was disclosed and nothing was read — the shape every no-survey entry passes. */
export const NO_REGIONS: RegionLookup = { rows: [], complete: true };

const nameOf = (id: string, lookup: RegionLookup): GroupingName => {
	const row = lookup.rows.find((r) => r.region_id === id);
	if (row) return { state: 'named', label: row.label, memberCount: row.member_count };
	return lookup.complete ? { state: 're-derived' } : { state: 'unchecked' };
};

/**
 * Build the readout from the trace — which covers EVERY stage, including the ones whose rows were
 * not returned. Intermediate stages are mostly not returned (the pipe carries ids, not rows), so
 * without the trace a composition is a black box with an answer at the end.
 */
export function buildReadout(response: QueryResponse, regions: RegionLookup = NO_REGIONS): Readout {
	const stages = response.trace?.stages ?? [];

	return {
		groupings: stages.flatMap((s) =>
			((s.disclosed_regions ?? []) as RegionDisclosure[]).map((d) => ({
				id: d.region_id,
				name: nameOf(d.region_id, regions),
			})),
		),
		stages: stages.map((s) => ({
			stage: s.stage,
			act: s.act,
			handed: Number(s.input_ids ?? 0),
			unusable: Number(s.input_unusable ?? 0),
		})),
	};
}

/** Every disclosed grouping id, for the route to resolve. Order-preserving, deduplicated. */
export function disclosedRegionIds(response: QueryResponse): string[] {
	const seen = new Set<string>();
	for (const s of response.trace?.stages ?? []) {
		for (const d of s.disclosed_regions ?? []) seen.add(d.region_id);
	}
	return [...seen];
}

/**
 * How many groupings the panel lists before it starts counting instead.
 *
 * `[measured against prod — 2026-08-20]` The flagship entry disclosed **970** distinct groupings
 * on the deployed (pre-Beat-0.5) substrate, because `survey` was joining its whole candidate pool
 * while `terms_applied` reported a width of 3. With the funnel honoured that same screen discloses
 * about fifteen, so this bound is a floor under a bad day rather than routine behaviour — but a
 * panel that renders one row per disclosure has to survive the bad day too.
 */
export const GROUPINGS_LISTED = 12;

/**
 * The groupings to show, and how many are not shown.
 *
 * **The remainder is returned, not dropped.** `legibility-is-never-bought-with-silent-omission`
 * is exactly the clause a "top 12" would break, and the count sentence above states the true
 * total, so the two halves have to agree: a reader told the answer came from 970 groupings and
 * shown 12 must be told the other 958 are there.
 */
export function listGroupings(
	readout: Readout,
	max: number = GROUPINGS_LISTED,
): { shown: Grouping[]; withheld: number } {
	return {
		shown: readout.groupings.slice(0, max),
		withheld: Math.max(0, readout.groupings.length - max),
	};
}

/** The remainder, said rather than shown as a bare number beside a truncated list. */
export function describeWithheld(withheld: number): string {
	return `${withheld} more ${withheld === 1 ? 'grouping is' : 'groupings are'} not listed here.`;
}

/**
 * Say where the answer came from, in the reader's words.
 *
 * No grouping at all is stated rather than counted: *"not drawn from any"* is a different sentence
 * from *"came from 0"*, and only the first is true of an entry that ran no funnel.
 */
export function describeReadout(readout: Readout): string {
	const n = readout.groupings.length;
	if (n === 0) return 'These were not drawn from any grouping of your work.';
	return `These came from ${n} ${n === 1 ? 'grouping' : 'groupings'} of your work.`;
}

/**
 * One grouping, in the reader's words.
 *
 * A grouping with no authored label is described by what it holds rather than given an invented
 * name — a machine-minted *"Grouping 3"* would be a label the reader never wrote, appearing in the
 * one panel whose whole job is to be honest about what is the machine's and what is theirs.
 */
export function describeGrouping(g: Grouping): string {
	switch (g.name.state) {
		case 'named': {
			const n = g.name.memberCount;
			const held = `${n} ${n === 1 ? 'resource' : 'resources'}`;
			return g.name.label ? `${g.name.label} · ${held}` : `An unnamed grouping · ${held}`;
		}
		case 're-derived':
			return 'This grouping has been re-derived.';
		default:
			return 'This grouping could not be looked up just now.';
	}
}
