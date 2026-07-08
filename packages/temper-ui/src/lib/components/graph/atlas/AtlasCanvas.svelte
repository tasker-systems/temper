<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { AtlasHome } from '$lib/types/generated/graph_home';
	import type { Focus, GraphFilters } from '$lib/graph/atlas/nav';
	import { attachCamera, type Camera } from '$lib/graph/atlas/camera';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/atlas/palette';
	import TierHome from './TierHome.svelte';
	import TierPanorama from './TierPanorama.svelte';
	import TierNeighborhood from './TierNeighborhood.svelte';

	interface Props {
		cogmapId: string | null;
		tier: number;
		focus: Focus;
		territories: TerritoryOverview | null;
		neighborhood: AtlasSubgraph | null;
		home: AtlasHome | null;
		filters: GraphFilters;
	}
	let { cogmapId, tier, focus, territories, neighborhood, home, filters }: Props = $props();

	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	const W = 1040;
	const H = 620;

	const seedId = $derived(focus.kind === 'node' ? focus.id : '');

	// Both the Tier-1 territory COMPOSITION drill (Beat D) and the Tier-2 node
	// neighborhood render as the force-graph; neither must fall through to a blank
	// canvas when the subgraph is empty (B4).
	const hasNeighbors = $derived(
		(tier === 1 || tier === 2) && !!neighborhood && neighborhood.nodes.length > 0
	);
	const emptyMessage = $derived(
		cogmapId && tier === 2
			? 'Node neighborhoods are not available in cogmap view yet — return to the map to explore its regions.'
			: tier === 2
				? 'This node has no mapped neighbors yet.'
				: tier === 1
					? 'This region has no linked resources yet.'
					: 'No data for this view.'
	);

	let svgEl: SVGSVGElement | undefined = $state();
	let viewportEl: SVGGElement | undefined = $state();
	let camera: Camera | undefined;

	// The parent keys this whole component on cogmapId|focus (Task 17), so onMount
	// re-fires on every re-scope and the d3-zoom transform resets (M6).
	onMount(() => {
		if (svgEl && viewportEl) {
			camera = attachCamera(svgEl, viewportEl, { min: MIN_ZOOM, max: MAX_ZOOM });
		}
	});
	onDestroy(() => camera?.destroy());
</script>

<div class="atlas-canvas" style={paletteStyleVars()}>
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Graph atlas">
		<rect x="0" y="0" width={W} height={H} fill={CANVAS_BG} />
		<g bind:this={viewportEl}>
			{#if !cogmapId && home}
				<TierHome {home} width={W} height={H} />
			{:else if tier === 0 && territories}
				<TierPanorama overview={territories} width={W} height={H} docTypes={filters.docTypes} />
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
