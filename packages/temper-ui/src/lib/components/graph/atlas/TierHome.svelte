<script lang="ts">
	// TierHome — Beat B "build / research" verb-lens Home (SPIKE / Task 1 prototype).
	// Two verb-CTAs over one Beat-A field panel: hazy rest → hover resolves a lens
	// (build = your contexts, research = the cogmaps you can reach) → click commits to
	// that lens only → a back affordance returns to neutral. Local state during the
	// spike (URL `?home` + real navigation are wired in Task 7). Reuses forceTerritories
	// + TerritoryCircle so Home speaks the same field language as the panorama.
	import type { LensedHome } from '$lib/types/generated/graph_home';
	import type { Territory } from '$lib/types/generated/graph_territory';
	import { forceTerritories } from '$lib/graph/atlas/layout/forceTerritories';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import TerritoryCircle from './marks/TerritoryCircle.svelte';

	interface Props {
		home: LensedHome;
		width: number;
		height: number;
	}
	let { home, width, height }: Props = $props();

	type Lens = 'build' | 'research';

	// Local lens machine (spike): committed wins, else hover, else neutral.
	let committed = $state<Lens | null>(null);
	let hover = $state<Lens | null>(null);
	const resolved = $derived<Lens | null>(committed ?? hover);

	const CTA_H = 104; // header band reserved for the two verb-CTAs
	const fieldSize = $derived({ width, height: Math.max(120, height - CTA_H) });

	// Map each lens's members to Territory shape so forceTerritories sizes them
	// (context/cogmap kinds weight by member_count).
	function buildTerritories(h: LensedHome): Territory[] {
		return h.build.map((c) => ({
			id: c.id,
			kind: 'context' as const,
			label: c.name,
			member_count: c.resource_count,
			salience: null,
			anchor_id: c.id
		}));
	}
	function researchTerritories(h: LensedHome): Territory[] {
		return h.research.map((m) => ({
			id: m.id,
			kind: 'cogmap' as const,
			label: m.name,
			member_count: m.region_count,
			salience: null,
			anchor_id: m.id
		}));
	}

	const buildPos = $derived(forceTerritories(buildTerritories(home), fieldSize));
	const researchPos = $derived(forceTerritories(researchTerritories(home), fieldSize));

	const buildMax = $derived(Math.max(1, ...home.build.map((c) => c.resource_count)));
	const researchMax = $derived(Math.max(1, ...home.research.map((m) => m.region_count)));
	const intensityFor = (mc: number, max: number) => 0.3 + 0.6 * Math.sqrt(Math.max(0, mc) / max);

	// §10.2 subtle per-scope tint: personal (@me) anchors at a base cool blue; each
	// team drifts to a distinct-but-cohesive hue in the blue→indigo band, so the two
	// scopes (and different teams) read apart without a rainbow. Keyed by owner_ref.
	const ownerRefById = $derived(new Map(home.build.map((c) => [c.id, c.owner_ref])));
	function buildTint(ownerRef: string): string {
		if (ownerRef === '@me') return 'hsl(200 44% 62%)';
		let h = 0;
		for (const ch of ownerRef) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
		return `hsl(${214 + (h % 5) * 12} 40% 64%)`;
	}

	// Research mirrors build in the WARM family: the universal/system kernel anchors at
	// base cogmap-orange; each team drifts across a red-orange→amber band. Scope comes
	// from the (spike) research owner_ref; a cogmap with no team is treated as universal.
	const researchScopeById = $derived(new Map(home.research.map((m) => [m.id, m.owner_ref ?? 'temper'])));
	function researchTint(scope: string): string {
		if (!scope.startsWith('+')) return 'hsl(34 80% 56%)'; // universal / system kernel
		let h = 0;
		for (const ch of scope) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
		return `hsl(${16 + (h % 5) * 9} 74% 56%)`;
	}

	// Group opacity per lens: rest = hazy union of both; previewing = the other fades
	// behind; committed = the other is gone.
	function lensOpacity(lens: Lens): number {
		if (resolved === null) return 0.26; // rest: ambient hazy union
		if (resolved === lens) return 1;
		return committed ? 0 : 0.07; // other: gone if committed, faint if previewing
	}

	function commit(lens: Lens) {
		committed = lens;
		hover = null;
	}
	function toNeutral() {
		committed = null;
		hover = null;
	}

	const TAGLINE: Record<Lens, string> = {
		build: 'your work, across your teams and personal space',
		research: 'the knowledge you can explore'
	};
	const LENS_TINT: Record<Lens, string> = {
		build: TERRITORY_TINTS.context,
		research: TERRITORY_TINTS.cogmap
	};

	const ctaW = $derived(Math.min(340, width * 0.34));
	const buildX = $derived(width / 2 - ctaW - 14);
	const researchX = $derived(width / 2 + 14);
	const ctaY = 22;

	function isActive(lens: Lens): boolean {
		return resolved === lens;
	}
</script>

<!-- Verb-CTAs -->
{#snippet cta(lens: Lens, x: number)}
	{@const active = isActive(lens)}
	<g
		role="button"
		tabindex="0"
		class="atlas-focusable cta"
		aria-label={`${lens} — ${TAGLINE[lens]}`}
		aria-pressed={committed === lens}
		onpointerenter={() => (hover = lens)}
		onpointerleave={() => (hover = null)}
		onfocus={() => (hover = lens)}
		onblur={() => (hover = null)}
		onclick={() => commit(lens)}
		onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && commit(lens)}
		style="cursor:pointer"
	>
		<rect
			{x}
			y={ctaY}
			width={ctaW}
			height={64}
			rx="12"
			fill={active ? LENS_TINT[lens] : 'rgba(255,255,255,0.02)'}
			fill-opacity={active ? 0.16 : 1}
			stroke={LENS_TINT[lens]}
			stroke-opacity={active ? 0.9 : 0.4}
			stroke-width={active ? 2 : 1}
		/>
		<text
			x={x + ctaW / 2}
			y={ctaY + 28}
			text-anchor="middle"
			fill={active ? LENS_TINT[lens] : '#c9ced9'}
			font-size="20"
			font-weight="700"
			letter-spacing="0.5">{lens}</text
		>
		<text
			x={x + ctaW / 2}
			y={ctaY + 48}
			text-anchor="middle"
			fill="#8b93a5"
			font-size="11">{TAGLINE[lens]}</text
		>
	</g>
{/snippet}

{@render cta('build', buildX)}
{@render cta('research', researchX)}

<!-- Back-to-neutral affordance once a lens is committed -->
{#if committed}
	<g
		role="button"
		tabindex="0"
		class="atlas-focusable"
		aria-label="Back to build / research choice"
		onclick={toNeutral}
		onkeydown={(e) => e.key === 'Enter' && toNeutral()}
		style="cursor:pointer"
	>
		<text x="20" y={ctaY + 40} fill="#8b93a5" font-size="13">← back</text>
	</g>
{/if}

<!-- The field: both lens layouts, cross-faded by lensOpacity -->
<g transform={`translate(0, ${CTA_H})`}>
	<g opacity={lensOpacity('build')} style="transition: opacity 260ms ease">
		{#each buildPos as t (t.id)}
			<TerritoryCircle
				x={t.x}
				y={t.y}
				r={t.r}
				kind="context"
				label={t.label}
				memberCount={t.member_count}
				showLabel={resolved === 'build'}
				intensity={intensityFor(t.member_count, buildMax)}
				tint={buildTint(ownerRefById.get(t.id) ?? '@me')}
			/>
		{/each}
	</g>
	<g opacity={lensOpacity('research')} style="transition: opacity 260ms ease">
		{#each researchPos as t (t.id)}
			<TerritoryCircle
				x={t.x}
				y={t.y}
				r={t.r}
				kind="cogmap"
				label={t.label}
				memberCount={t.member_count}
				showLabel={resolved === 'research'}
				intensity={intensityFor(t.member_count, researchMax)}
				tint={researchTint(researchScopeById.get(t.id) ?? 'temper')}
			/>
		{/each}
	</g>
</g>
