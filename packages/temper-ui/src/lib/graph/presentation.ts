import type { ResourceView } from '$lib/types/generated/resource_view';
import { relativeTime } from './atlas/relativeTime';
import type { GraphNode, NodeArm } from './model';

/**
 * How a node describes itself in the panels beside the canvas.
 *
 * Pure, and separate from `model.ts`, because these are **sentences shown to a reader** rather
 * than the data behind a mark — so they are tested as strings, and the vocabulary rule that
 * governs the readout governs them too.
 */

/**
 * Where a resource lives, in the reader's terms.
 *
 * A resource is homed by exactly one anchor, so the unused half is **absent from the wire** rather
 * than null. Reading `context_ref` first and falling through to `cogmap_name` is therefore
 * exhaustive rather than a preference — but a row that carries neither is still possible from a
 * projection that did not fill them, and it says so instead of rendering an empty cell.
 */
export function whereOf(row: ResourceView): string {
	return row.context_ref ?? row.cogmap_name ?? 'home not reported';
}

/**
 * What put this node on screen, said without naming an act.
 *
 * `no-internal-vocabulary-is-load-bearing` reaches here too: the reader is told *followed on from
 * your work*, never *"reached by `follow-from`"*. The three phrases are the three the bound line
 * uses, so the same partition reads the same way in both places.
 */
export function describeArm(arm: NodeArm): string {
	switch (arm) {
		case 'seed':
			return 'In the places you asked about';
		case 'survey':
			return 'From your places';
		default:
			return 'Followed on from your work';
	}
}

/**
 * The metadata rows a hover card carries — **N2**, and the whole point of it.
 *
 * `[N2 — 2026-08-20]` Hover used to carry the title and little else. Every node here holds its
 * whole `ResourceView`, so where it lives, what stage it is at and when it last moved are already
 * in hand: this is a projection of a row the read returned, not a second read.
 *
 * A row is **omitted when the field is absent** rather than rendered as a dash. An empty value in
 * a metadata list reads as *this resource has no stage*, which is a claim; leaving the row out
 * says only that nothing was reported.
 */
export function nodeMeta(
	node: GraphNode,
	now: Date = new Date(),
): { label: string; value: string }[] {
	const rows = [{ label: 'in', value: whereOf(node.resource) }];
	const stage = node.resource.managed_meta?.['temper-stage'];
	if (stage) rows.push({ label: 'stage', value: String(stage) });
	if (node.resource.updated) {
		rows.push({ label: 'updated', value: relativeTime(node.resource.updated, now) });
	}
	rows.push({ label: 'reached', value: describeArm(node.arm).toLowerCase() });
	return rows;
}

/**
 * Mark radius from degree.
 *
 * Degree is counted over the **deduped** edge set, which is why this is safe to size on: the
 * highest-degree node in the measured walk carried 98 `via` entries over 25 distinct edges, so
 * sizing on the raw count would have inflated it fourfold and made the hub look like an outlier
 * of the reader's corpus rather than of the walk's bookkeeping.
 */
export const nodeRadius = (degree: number): number => 7 + Math.min(9, degree * 0.6);
