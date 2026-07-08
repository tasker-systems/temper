<script lang="ts">
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import AtlasLegend from '$lib/components/graph/atlas/AtlasLegend.svelte';
	import AtlasCrumb from '$lib/components/graph/atlas/AtlasCrumb.svelte';
	import TrailRail from '$lib/components/graph/atlas/TrailRail.svelte';
	import HomeA11yList from '$lib/components/graph/atlas/HomeA11yList.svelte';
	import CompositionA11yList from '$lib/components/graph/atlas/CompositionA11yList.svelte';
	import { selectedElement, type SelectedElement } from '$lib/graph/atlas/nav';
	import type { AtlasViewData } from '$lib/graph/atlas/viewData';
	import { navigating, page } from '$app/stores';

	// `selectionOverride` is a dev-harness affordance: prod derives the selection from
	// the live `?sel=` URL param (it must, so ephemeral replaceState edge-selects that
	// don't re-run `load` still register), but the /dev/atlas harness renders at a static
	// URL with no `?sel=`. Passing the fixture's captured `selection` here lets the harness
	// replay `?sel=`-driven selections (context-node + edge rails) that the URL can't carry.
	// Prod never passes it, so production behavior is unchanged.
	let {
		data,
		selectionOverride = null
	}: { data: AtlasViewData; selectionOverride?: SelectedElement | null } = $props();

	// M6: keying AtlasCanvas on the scoped view remounts it on re-scope, resetting the camera.
	// Selection (`?sel`) is deliberately excluded — selecting an edge must not remount the canvas.
	const viewKey = $derived(
		`${data.cogmapId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
	const selection = $derived(selectionOverride ?? selectedElement(data.focus, $page.url));
	const subgraph = $derived(data.neighborhood ?? null);
	// The TrailRail derives node/edge detail from the loaded subgraph, so an edge
	// selection and neighbor rendering need a populated subgraph. A resource LEAF is
	// the exception: even with no mapped neighbors (empty subgraph) it opens a working
	// rail off its resourceRow (meta) + trail (history), which the server loads for any
	// node selection. So the rail shows when the subgraph has nodes OR a selected node
	// carries a resourceRow. Still suppressed: no selection, and cogmap-scoped nodes
	// (B3 — no resourceRow there), where the canvas carries the explanatory message.
	const hasPanelData = $derived(
		(subgraph !== null && subgraph.nodes.length > 0) ||
			(selection.kind === 'node' && data.resourceRow !== null)
	);
	const seedTitle = $derived(
		data.focus.kind === 'node' && subgraph
			? (subgraph.nodes.find((n) => n.id === (data.focus as { id: string }).id)?.title ?? null)
			: null
	);
	// Show the loading veil only for real view-loads — a scope or focus change that
	// remounts the canvas — not for ephemeral replaceState navigations (filter
	// toggle, edge select, panel close) which keep the same cogmap/focus. Those
	// still run `load`, so an unconditional $navigating veil would flash on every
	// minor interaction.
	const scopeKey = (u: URL) => `${u.searchParams.get('cogmap') ?? 'home'}|${u.searchParams.get('focus') ?? ''}`;
	const isViewLoad = $derived(!!$navigating?.to && scopeKey($navigating.to.url) !== scopeKey($page.url));
</script>

<div class="atlas-page">
	<div class="top-bar">
		<AtlasCrumb
			cogmapName={data.cogmapName}
			focusPath={data.focusPath}
			crumbTerritory={data.crumbTerritory}
			{seedTitle}
			cogmapId={data.cogmapId}
			scopeFilter={data.scopeFilter}
		/>
	</div>

	<div class="canvas-wrap">
		{#if data.home}
			<HomeA11yList home={data.home} />
		{/if}
		{#if subgraph && subgraph.nodes.length > 0}
			<CompositionA11yList {subgraph} />
		{/if}
		{#if isViewLoad}
			<div class="loading-veil" role="status" aria-live="polite">Loading…</div>
		{/if}
		{#key viewKey}
			<AtlasCanvas
				cogmapId={data.cogmapId}
				tier={data.tier}
				focus={data.focus}
				territories={data.territories}
				neighborhood={data.neighborhood}
				home={data.home}
				filters={data.filters}
			/>
		{/key}
		{#if selection.kind !== 'none' && hasPanelData}
			<TrailRail {selection} {subgraph} trail={data.trail} resourceRow={data.resourceRow} />
		{/if}
	</div>

	<div class="bottom-bar"><AtlasLegend /></div>
</div>

<style>
	.atlas-page {
		display: grid;
		grid-template-rows: auto 1fr auto;
		height: 100%;
		min-height: 0;
	}
	.top-bar {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 8px 14px;
		border-bottom: 1px solid rgba(255, 255, 255, 0.06);
		min-width: 0;
	}
	.canvas-wrap {
		position: relative;
		min-width: 0;
		min-height: 0;
		/* The canvas fits-to-box (AtlasCanvas svg is height:100%), but clip here as
		   defense-in-depth so nothing — canvas or an overlaid rail — can ever bleed
		   into the legend band below. */
		overflow: hidden;
	}
	/* TrailRail's own stylesheet has no `position` — it previously relied on the old
	   3-column grid's `auto` column to sit at the right. Now that it's nested inside
	   `.canvas-wrap`, pin it as an absolutely-positioned right rail so it overlays the
	   canvas instead of stacking below it in normal flow. */
	.canvas-wrap > :global(.trail-rail) {
		position: absolute;
		top: 0;
		right: 0;
		bottom: 0;
		z-index: 3;
	}
	.bottom-bar {
		border-top: 1px solid rgba(255, 255, 255, 0.06);
		/* Bound the band and scroll internally when the legend is open, so an
		   expanded legend lays out inside its band instead of growing up into the
		   canvas (Beat-2a regression L1). The horizontal legend layout keeps it a
		   thin strip in the common case; this caps the worst case. */
		max-height: 42vh;
		overflow: auto;
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
</style>
