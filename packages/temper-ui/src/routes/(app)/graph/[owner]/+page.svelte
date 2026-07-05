<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import AtlasLegend from '$lib/components/graph/atlas/AtlasLegend.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';
	import CogmapCrumb from '$lib/components/graph/atlas/CogmapCrumb.svelte';
	import SearchAccelerator from '$lib/components/graph/atlas/SearchAccelerator.svelte';
	import TrailRail from '$lib/components/graph/atlas/TrailRail.svelte';
	import { selectedElement } from '$lib/graph/atlas/nav';
	import { navigating, page } from '$app/stores';

	let { data }: { data: PageData } = $props();

	// M6: keying AtlasCanvas on the scoped view remounts it on re-scope, resetting the camera.
	// Selection (`?sel`) is deliberately excluded — selecting an edge must not remount the canvas.
	const viewKey = $derived(
		`${data.teamId ?? data.cogmapId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
	const selection = $derived(selectedElement(data.focus, $page.url));
	const subgraph = $derived(data.neighborhood ?? null);
	// The TrailRail derives node/edge detail from the loaded subgraph, so it only has
	// real content when a subgraph with nodes is present. Gating on this suppresses the
	// thin/empty panel for cogmap-scoped nodes (no subgraph, B3) and empty neighborhoods
	// (B4) — the canvas carries the explanatory message in those cases instead.
	const hasPanelData = $derived(subgraph !== null && subgraph.nodes.length > 0);
	// Show the loading veil only for real view-loads — a scope or focus change that
	// remounts the canvas — not for ephemeral replaceState navigations (filter
	// toggle, edge select, panel close) which keep the same team/cogmap/focus. Those
	// still run `load`, so an unconditional $navigating veil would flash on every
	// minor interaction.
	const scopeKey = (u: URL) =>
		`${u.searchParams.get('team') ?? u.searchParams.get('cogmap') ?? 'home'}|${u.searchParams.get('focus') ?? ''}`;
	const isViewLoad = $derived(!!$navigating?.to && scopeKey($navigating.to.url) !== scopeKey($page.url));
</script>

<div class="atlas-page">
	<aside class="dock">
		{#if data.teamId}
			<SearchAccelerator teamId={data.teamId} />
		{/if}
		{#if data.scope}
			<ScopeBar scope={data.scope} filters={data.filters} />
		{:else if data.cogmapId}
			<CogmapCrumb name={data.cogmapName ?? 'Cognitive map'} />
		{:else}
			<nav class="scope-bar home">Atlas · your teams</nav>
		{/if}
		<AtlasLegend />
	</aside>
	<div class="canvas-wrap">
		{#if isViewLoad}
			<div class="loading-veil" role="status" aria-live="polite">Loading…</div>
		{/if}
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
				filters={data.filters}
			/>
		{/key}
	</div>
	{#if selection.kind !== 'none' && hasPanelData}
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
	.loading-veil {
		position: absolute;
		top: 12px;
		left: 50%;
		transform: translateX(-50%);
		z-index: 2;
		padding: 4px 14px;
		border-radius: 12px;
		background: rgba(20, 23, 29, 0.85);
		border: 1px solid rgba(255, 255, 255, 0.08);
		color: var(--color-quiet-ink, #c9ced9);
		font-size: 12px;
		letter-spacing: 0.04em;
		pointer-events: none;
	}
	.scope-bar.home {
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
</style>
