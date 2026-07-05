// trail.ts
import type { EventTrail } from '$lib/types/generated/element_trail';

export interface TrailRow {
	/** The event's unique id (`kb_events.event_id`) — the stable list key. Two
	 *  events can share actor+time+kind (e.g. a batch mutation), so keying a
	 *  render on those fields collides and crashes the panel; key on this. */
	id: string;
	kind: string;
	actor: string;
	occurredAt: string;
	confidence: string | null;
}

/** Humanize an R5 EventTrail into display rows, newest-first. `kind` is the
 *  canonical dotted event type (e.g. "relationship.reweighted"); we show the
 *  trailing segment title-cased. Confidence is normalized (absent → null). */
export function trailModel(trail: EventTrail): TrailRow[] {
	return [...trail.events]
		.reverse()
		.map((e) => ({
			id: e.event_id,
			kind: humanizeKind(e.kind),
			actor: e.actor_entity_id,
			occurredAt: e.occurred_at,
			confidence: e.confidence ?? null
		}));
}

function humanizeKind(kind: string): string {
	const tail = kind.split('.').pop() ?? kind;
	return tail.charAt(0).toUpperCase() + tail.slice(1).replace(/_/g, ' ');
}
