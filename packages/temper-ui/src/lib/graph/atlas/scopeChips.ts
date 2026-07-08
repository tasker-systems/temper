/**
 * Home scope-filter chips (Beat C): the committed Home lens can be narrowed to
 * one owner-scope via `?scope` (nav.ts's `buildScopeFilterUrl`). This derives the
 * chip set — the distinct owner-scopes actually present in the lens's bodies —
 * so TierHome doesn't hand-roll dedup/sort logic. Pure; wiring lives in TierHome.
 */

/** The distinct owner-scopes present in a lens's bodies, sorted for a stable chip order. */
export function deriveScopeChips(bodies: { owner_ref: string }[]): string[] {
	return [...new Set(bodies.map((b) => b.owner_ref))].sort();
}
