<script lang="ts">
	// TierHome — Beat B "build / research" verb-lens Home.
	// Two verb-CTAs over one Beat-A field panel: hazy union rest → hover resolves a lens
	// (build = your contexts, research = the cogmaps you can reach) → click commits to
	// that lens only (`?home` in the URL) → a back affordance returns to neutral. The
	// committed lens is URL-derived; hover is an ephemeral local preview that keeps the
	// field crisp across the (background) load round-trip. Reuses forceTerritories +
	// TerritoryCircle so Home speaks the same field language as the panorama.
	import type { AtlasHome } from '$lib/types/generated/graph_home';
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import {
		parseHomeLens,
		buildHomeLensUrl,
		clearHomeLensUrl,
		buildCogmapUrl,
		type HomeLens
	} from '$lib/graph/atlas/nav';
	import {
		buildLensTerritories,
		researchLensTerritories,
		layoutHomeLens
	} from '$lib/graph/atlas/layout/homeLayout';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import TerritoryCircle from './marks/TerritoryCircle.svelte';

	interface Props {
		home: AtlasHome;
		width: number;
		height: number;
	}
	let { home, width, height }: Props = $props();

	type Lens = HomeLens;

	// The committed lens lives in the URL (`?home`); Back returns to neutral. Hover is
	// an ephemeral local preview. `goto` (not shallow pushState — which leaves `page.url`
	// stale) updates `$page.url` reactively so the field resolves, and gives real Back
	// history. Home data is lens-independent, so the load re-run is a cheap re-read.
	const committed = $derived<Lens | null>(parseHomeLens($page.url));
	let hover = $state<Lens | null>(null);
	const resolved = $derived<Lens | null>(committed ?? hover);

	const CTA_H = 104; // header band reserved for the two verb-CTAs
	const fieldSize = $derived({ width, height: Math.max(120, height - CTA_H) });

	const buildPos = $derived(layoutHomeLens(buildLensTerritories(home), fieldSize));
	const researchPos = $derived(layoutHomeLens(researchLensTerritories(home), fieldSize));

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
		return `hsl(${200 + (h % 8) * 8} 40% 64%)`; // 8 buckets across a cool blue→indigo band
	}

	// Research mirrors build in the WARM family: the universal/system kernel (owner_ref
	// not starting '+') anchors at base cogmap-orange; each team drifts across a
	// red-orange→amber band keyed by its +slug.
	const researchScopeById = $derived(new Map(home.research.map((m) => [m.id, m.owner_ref])));
	function researchTint(scope: string): string {
		if (!scope.startsWith('+')) return 'hsl(34 80% 56%)'; // universal / system kernel
		let h = 0;
		for (const ch of scope) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
		return `hsl(${12 + (h % 8) * 6} 74% 56%)`; // 8 buckets across a red-orange→amber band
	}

	// Group opacity per lens: rest = hazy union of both; previewing = the other fades
	// behind; committed = the other is gone.
	function lensOpacity(lens: Lens): number {
		if (resolved === null) return 0.26; // rest: ambient hazy union
		if (resolved === lens) return 1;
		return committed ? 0 : 0.07; // other: gone if committed, faint if previewing
	}

	function commit(lens: Lens) {
		// Keep `hover` set: `goto` re-runs the load asynchronously, so until `committed`
		// (URL-derived) catches up, the hover preview keeps the chosen lens crisp — no
		// flash back to the hazy rest during the round-trip.
		goto(buildHomeLensUrl($page.url, lens), { keepFocus: true, noScroll: true });
	}
	function toNeutral() {
		// Clear hover so the field returns to neutral immediately, not after the load.
		hover = null;
		goto(clearHomeLensUrl($page.url), { keepFocus: true, noScroll: true });
	}

	// Body navigation. Research → the cogmap panorama (Beat A). Build → the owner's
	// vault (temporary destination §10.4; Atlas-native contexts panorama is Beat C).
	function enterContext(ownerRef: string) {
		goto(`/vault/${ownerRef}`);
	}
	function enterCogmap(id: string) {
		goto(buildCogmapUrl($page.url, id));
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
		onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), commit(lens))}
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
		onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toNeutral())}
		style="cursor:pointer"
	>
		<text x="20" y={ctaY + 40} fill="#8b93a5" font-size="13">← back</text>
	</g>
{/if}

<!-- The field: both lens layouts, cross-faded by lensOpacity -->
<g transform={`translate(0, ${CTA_H})`}>
	<g
		opacity={lensOpacity('build')}
		style={`transition: opacity 260ms ease; pointer-events: ${committed === 'build' ? 'auto' : 'none'}`}
	>
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
				onEnter={committed === 'build'
					? () => enterContext(ownerRefById.get(t.id) ?? '@me')
					: undefined}
			/>
		{/each}
	</g>
	<g
		opacity={lensOpacity('research')}
		style={`transition: opacity 260ms ease; pointer-events: ${committed === 'research' ? 'auto' : 'none'}`}
	>
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
				onEnter={committed === 'research' ? () => enterCogmap(t.id) : undefined}
			/>
		{/each}
	</g>
		<!-- Empty-state for a committed lens with nothing in it. -->
		{#if committed === 'build' && buildPos.length === 0}
			<text x={width / 2} y={fieldSize.height / 2} text-anchor="middle" fill="#8b93a5" font-size="14">
				You don't have any contexts to build in yet.
			</text>
		{:else if committed === 'research' && researchPos.length === 0}
			<text x={width / 2} y={fieldSize.height / 2} text-anchor="middle" fill="#8b93a5" font-size="14">
				There are no cognitive maps you can reach yet.
			</text>
		{/if}
</g>
