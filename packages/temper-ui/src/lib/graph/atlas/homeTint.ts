/**
 * Home field tints (Beat B, §10.2): pure presentation helpers that give each
 * Home territory its fill intensity and per-scope hue. Cool vs warm splits the
 * two verb-lenses — build tints run a cool blue→indigo band (personal `@me`
 * anchors at a base blue, each team drifts to a distinct-but-cohesive hue), and
 * research tints run a warm red-orange→amber band (the universal/system kernel
 * anchors at base cogmap-orange, each `+team` drifts across the band). So the
 * two scopes (and different teams within them) read apart without a rainbow.
 * Pure; the field-effect + wiring live in TierHome.
 */

/** Fill intensity from a member count, floored at 0.3 and rising by sqrt to 0.9 at max. */
export const intensityFor = (mc: number, max: number): number =>
	0.3 + 0.6 * Math.sqrt(Math.max(0, mc) / max);

/** Deterministic 31-based string hash into a u32; shared by both tint bands. */
function hashScope(s: string): number {
	let h = 0;
	for (const ch of s) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
	return h;
}

/**
 * Build tint (cool blue→indigo): `@me` anchors at a base cool blue; each team
 * drifts to one of 8 distinct-but-cohesive hues in the blue→indigo band, keyed
 * by owner_ref.
 */
export function buildTint(ownerRef: string): string {
	if (ownerRef === '@me') return 'hsl(200 44% 62%)';
	const h = hashScope(ownerRef);
	return `hsl(${200 + (h % 8) * 8} 40% 64%)`; // 8 buckets across a cool blue→indigo band
}

/**
 * Research tint (warm red-orange→amber): the universal/system kernel (owner_ref
 * not starting `+`) anchors at base cogmap-orange; each team drifts across a
 * red-orange→amber band keyed by its `+slug`.
 */
export function researchTint(scope: string): string {
	if (!scope.startsWith('+')) return 'hsl(34 80% 56%)'; // universal / system kernel
	const h = hashScope(scope);
	return `hsl(${12 + (h % 8) * 6} 74% 56%)`; // 8 buckets across a red-orange→amber band
}
