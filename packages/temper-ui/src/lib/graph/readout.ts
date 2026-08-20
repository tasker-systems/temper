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
 * One grouping of the reader's work that the answer drew on.
 *
 * **It carries an id and nothing else, and the missing field is deliberate.** The blend the substrate
 * orders these by is an OPEN ruling — it spans `[-0.57, 1.05]`, so it can be negative and it exceeds
 * one — and the standing consequence for this surface is that the number is **never presented to the
 * reader as a score**. The readout may say which groupings the answer drew on and in what order; it
 * may not print the number or imply a calibrated scale. Keeping the field off the type is what makes
 * that unforgettable rather than merely documented.
 *
 * The id is carried because a reference to a grouping is not durable — the substrate mints a new one
 * whenever a member set changes — so the surface needs it to notice a reference that no longer
 * resolves and render *"this grouping has been re-derived"*, never an error and never the reader's
 * mistake.
 */
export interface Grouping {
	id: string;
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
 * Build the readout from the trace — which covers EVERY stage, including the ones whose rows were
 * not returned. Intermediate stages are mostly not returned (the pipe carries ids, not rows), so
 * without the trace a composition is a black box with an answer at the end.
 */
export function buildReadout(response: QueryResponse): Readout {
	const stages = response.trace?.stages ?? [];

	return {
		groupings: stages.flatMap((s) =>
			((s.disclosed_regions ?? []) as RegionDisclosure[]).map((d) => ({ id: d.region_id })),
		),
		stages: stages.map((s) => ({
			stage: s.stage,
			act: s.act,
			handed: Number(s.input_ids ?? 0),
			unusable: Number(s.input_unusable ?? 0),
		})),
	};
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
