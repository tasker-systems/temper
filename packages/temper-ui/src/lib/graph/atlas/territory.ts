// territory.ts
import type { Territory } from '$lib/types/generated/graph_territory';

/** A territory with no members — rendered as a de-emphasized ghost (L3). */
export function isEmptyTerritory(t: Territory): boolean {
	return t.member_count === 0;
}
