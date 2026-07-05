<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview, TerritorySlice } from '$lib/types/generated/graph_territory';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { HomeCogmap, HomeTeam } from '$lib/types/generated/graph_home';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import type { Focus, GraphFilters } from '$lib/graph/atlas/nav';
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
		filters: GraphFilters;
	}
	let { teamId, cogmapId, tier, focus, territories, slice, neighborhood, teams, cogmaps, zones, filters }: Props =
		$props();

	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	const W = 1040;
	const H = 620;

	const seedId = $derived(focus.kind === 'node' ? focus.id : '');

	// A Tier-2 neighborhood with no nodes must not fall through to a blank canvas
	// (B4). Cogmap scope has no neighborhood read at all, so a node drilled inside a
	// cogmap gets a scope-aware message instead of the generic "No data" (B3).
	const hasNeighbors = $derived(tier === 2 && !!neighborhood && neighborhood.nodes.length > 0);
	const emptyMessage = $derived(
		cogmapId && tier === 2
			? 'Node neighborhoods are not available in cogmap view yet — return to the map to explore its regions.'
			: tier === 2
				? 'This node has no mapped neighbors yet.'
				: 'No data for this view.'
	);

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
				<TierPanorama overview={territories} {zones} width={W} height={H} docTypes={filters.docTypes} />
			{:else if tier === 1 && slice}
				<TierTerritory {slice} width={W} height={H} />
			{:else if hasNeighbors && neighborhood}
				<TierNeighborhood subgraph={neighborhood} {seedId} width={W} height={H} docTypes={filters.docTypes} />
			{:else}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">{emptyMessage}</text>
			{/if}
		</g>
	</svg>
</div>

<style>
	.atlas-canvas {
		width: 100%;
		height: 100%;
		min-height: 0;
	}
	/* Fill the (bounded) canvas row and letterbox via the viewBox's default
	   `preserveAspectRatio: xMidYMid meet`, so the whole map stays visible and the
	   svg never exceeds its 1fr grid row. The previous `height: auto` sized the svg
	   to the viewBox's intrinsic aspect ratio, overflowing short viewports and
	   pushing the bottom-bar legend off-screen (Beat-2a regressions L2/short-height).
	   The d3 camera still zooms/pans for detail. */
	.atlas-canvas svg {
		display: block;
		width: 100%;
		height: 100%;
	}
</style>
