<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { HomeCogmap, HomeTeam } from '$lib/types/generated/graph_home';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import type { Focus } from '$lib/graph/atlas/nav';
	import { attachCamera, type Camera } from '$lib/graph/atlas/camera';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/atlas/palette';
	import TierHome from './TierHome.svelte';
	import TierPanorama from './TierPanorama.svelte';
	import TierTerritory from './TierTerritory.svelte';
	import TierNeighborhood from './TierNeighborhood.svelte';

	interface Props {
		teamId: string | null;
		cogmapId: string | null;
		tier: number;
		focus: Focus;
		territories: TerritoryOverview | null;
		slice: TerritorySlice | null;
		neighborhood: AtlasSubgraph | null;
		teams: HomeTeam[] | null;
		cogmaps: HomeCogmap[] | null;
		zones: TeamZone[];
	}
	let { teamId, cogmapId, tier, focus, territories, slice, neighborhood, teams, cogmaps, zones }: Props =
		$props();

	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	const W = 1040;
	const H = 620;

	const seedId = $derived(focus.kind === 'node' ? focus.id : '');

	let svgEl: SVGSVGElement | undefined = $state();
	let viewportEl: SVGGElement | undefined = $state();
	let camera: Camera | undefined;

	// The parent keys this whole component on teamId|focus (Task 17), so onMount
	// re-fires on every re-scope and the d3-zoom transform resets (M6).
	onMount(() => {
		if (svgEl && viewportEl) {
			camera = attachCamera(svgEl, viewportEl, { min: MIN_ZOOM, max: MAX_ZOOM });
		}
	});
	onDestroy(() => camera?.destroy());
</script>

<div class="atlas-canvas" style={paletteStyleVars()}>
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Team graph atlas">
		<rect x="0" y="0" width={W} height={H} fill={CANVAS_BG} />
		<g bind:this={viewportEl}>
			{#if !teamId && !cogmapId && teams}
				<TierHome {teams} cogmaps={cogmaps ?? []} width={W} height={H} />
			{:else if tier === 0 && territories}
				<TierPanorama overview={territories} {zones} width={W} height={H} />
			{:else if tier === 1 && slice}
				<TierTerritory {slice} width={W} height={H} />
			{:else if tier === 2 && neighborhood}
				<TierNeighborhood subgraph={neighborhood} {seedId} width={W} height={H} />
			{:else}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">No data for this view.</text>
			{/if}
		</g>
	</svg>
</div>

<style>
	.atlas-canvas {
		width: 100%;
		height: 100%;
	}
	.atlas-canvas svg {
		display: block;
		width: 100%;
		height: auto;
	}
</style>
