<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { TerritoryOverview } from '$lib/types/generated/graph_territory';
	import type { TeamZone } from '$lib/types/generated/graph_scope';
	import { attachCamera, type Camera } from '$lib/graph/atlas/camera';
	import { paletteStyleVars } from '$lib/graph/atlas/palette';
	import TierPanorama from './TierPanorama.svelte';

	interface Props {
		tier: number;
		territories: TerritoryOverview | null;
		zones: TeamZone[];
	}
	let { tier, territories, zones }: Props = $props();

	const MIN_ZOOM = 0.3;
	const MAX_ZOOM = 4;
	const W = 1040;
	const H = 620;

	let svgEl: SVGSVGElement | undefined = $state();
	let viewportEl: SVGGElement | undefined = $state();
	let camera: Camera | undefined;

	onMount(() => {
		if (svgEl && viewportEl) {
			camera = attachCamera(svgEl, viewportEl, { min: MIN_ZOOM, max: MAX_ZOOM });
		}
	});
	onDestroy(() => camera?.destroy());
</script>

<div class="atlas-canvas" style={paletteStyleVars()}>
	<svg bind:this={svgEl} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Team graph atlas">
		<rect x="0" y="0" width={W} height={H} fill="#1b1e26" />
		<g bind:this={viewportEl}>
			{#if tier === 0 && territories}
				<TierPanorama overview={territories} {zones} width={W} height={H} />
			{:else}
				<text x={W / 2} y={H / 2} text-anchor="middle" fill="#7d8496" font-size="14">
					Tier {tier} view lands in Chunk C2
				</text>
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
