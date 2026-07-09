<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';
	import type { AtlasHome } from '$lib/types/generated/graph_home';
	import type { ContextPanorama } from '$lib/types/generated/graph_context';
	import type { Focus, GraphFilters } from '$lib/graph/atlas/nav';
	import { attachCamera, type Camera } from '$lib/graph/atlas/camera';
	import { CANVAS_BG, paletteStyleVars } from '$lib/graph/atlas/palette';
	import TierHome from './TierHome.svelte';
	import TierPanorama from './TierPanorama.svelte';
	import TierNeighborhood from './TierNeighborhood.svelte';
	import ResidualTray from './ResidualTray.svelte';

	interface Props {
		cogmapId: string | null;
		/** Beat E `?context` scope; null on every other door. Selects the context branch. */
		contextSlug: string | null;
		/** Beat E Tier-0 payload (container territories + residual tray) for the context door. */
		panorama: ContextPanorama | null;
		tier: number;
		focus: Focus;
		territories: TerritoryOverview | null;
		neighborhood: AtlasSubgraph | null;
		home: AtlasHome | null;
		filters: GraphFilters;
	}
	let { cogmapId, contextSlug, panorama, tier, focus, territories, neighborhood, home, filters }: Props =
		$props();

	// Beat E: a context panorama reuses TierPanorama by adapting its container territories into
	// the TerritoryOverview shape (no orphan cogmaps, no bridges). The residual tray is NOT part
	// of this — it is a doorway lifted out of the camera (see the HTML sibling below).
	const contextOverview = $derived<TerritoryOverview | null>(
		panorama ? { territories: panorama.containers, orphan_nodes: [], bridges: [] } : null
	);
	const residual = $derived(panorama?.residual ?? null);

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
			{#if !cogmapId && !contextSlug && home}
				<TierHome {home} width={W} height={H} />
			{:else if tier === 0 && contextSlug && contextOverview}
				<TierPanorama overview={contextOverview} width={W} height={H} docTypes={filters.docTypes} />
			{:else if tier === 0 && territories}
				<TierPanorama overview={territories} width={W} height={H} docTypes={filters.docTypes} />
			{:else if hasNeighbors && neighborhood}
				<TierNeighborhood
					subgraph={neighborhood}
					{seedId}
					width={W}
					height={H}
					docTypes={filters.docTypes}
					coreHome={contextSlug ? 'context' : 'cogmap'}
				/>
			{:else}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">{emptyMessage}</text>
			{/if}
		</g>
	</svg>
	<!--
		The residual tray is a doorway, not a field landmark (spec §7): it lives in page chrome
		as an HTML sibling of the <svg>, OUTSIDE the camera-transformed <g>, so it never pans or
		scales. It shows only on the context Tier-0 panorama, and only when residue exists — a
		well-edged context absorbs everything into containers and the tray vanishes.
	-->
	{#if tier === 0 && contextSlug && residual && residual.buckets.length > 0}
		<ResidualTray buckets={residual.buckets} groupKey={residual.group_key} width={W} />
	{/if}
</div>

<style>
	.atlas-canvas {
		display: flex;
		flex-direction: column;
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
		/* Fill the column; when the residual tray sibling is present it takes its natural
		   height below and the svg letterboxes into the remaining space. */
		flex: 1 1 auto;
		min-height: 0;
	}

	/* The residual-tray doorway sits below the map, fixed to the page — it does not pan or
	   scale with the d3 camera (spec §7). */
	.atlas-canvas :global(.residual-tray) {
		flex: 0 0 auto;
	}
</style>
