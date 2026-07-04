<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';
	import SearchAccelerator from '$lib/components/graph/atlas/SearchAccelerator.svelte';
	import TrailRail from '$lib/components/graph/atlas/TrailRail.svelte';
	import { selectedElement } from '$lib/graph/atlas/nav';
	import { page } from '$app/stores';

	let { data }: { data: PageData } = $props();

	// M6: keying AtlasCanvas on the scoped view remounts it on re-scope, resetting the camera.
	// Selection (`?sel`) is deliberately excluded — selecting an edge must not remount the canvas.
	const viewKey = $derived(
		`${data.teamId ?? data.cogmapId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
	const selection = $derived(selectedElement(data.focus, $page.url));
	const subgraph = $derived(data.neighborhood ?? null);
</script>

<div class="atlas-page">
	<aside class="dock">
		{#if data.teamId}
			<SearchAccelerator teamId={data.teamId} />
		{/if}
		{#if data.scope}
			<ScopeBar scope={data.scope} />
		{:else}
			<nav class="scope-bar home">Atlas · your teams</nav>
		{/if}
	</aside>
	<div class="canvas-wrap">
		{#key viewKey}
			<AtlasCanvas
				teamId={data.teamId}
				cogmapId={data.cogmapId}
				tier={data.tier}
				focus={data.focus}
				territories={data.territories}
				slice={data.slice}
				neighborhood={data.neighborhood}
				teams={data.teams}
				cogmaps={data.cogmaps}
				zones={data.scope?.zones ?? []}
			/>
		{/key}
	</div>
	{#if selection.kind !== 'none'}
		<TrailRail {selection} {subgraph} trail={data.trail} resourceRow={data.resourceRow} />
	{/if}
</div>

<style>
	.atlas-page {
		display: grid;
		grid-template-columns: 232px 1fr auto;
		height: 100%;
		min-height: 0;
	}
	.dock {
		border-right: 1px solid rgba(255, 255, 255, 0.06);
		overflow-y: auto;
	}
	.canvas-wrap {
		position: relative;
		min-width: 0;
	}
	.scope-bar.home {
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
</style>
