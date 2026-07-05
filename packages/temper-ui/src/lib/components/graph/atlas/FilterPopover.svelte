<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { EdgeKind } from '$lib/types/generated/graph';
	import { buildFiltersUrl, activeFilterCount, type GraphFilters } from '$lib/graph/atlas/nav';
	import { DOC_TYPE_HUES, type AtlasDocType } from '$lib/graph/atlas/palette';

	interface Props {
		filters: GraphFilters;
	}
	let { filters }: Props = $props();

	const EDGE_KIND_OPTIONS: EdgeKind[] = ['contains', 'leads_to', 'express', 'near'];
	const DOC_TYPE_OPTIONS = Object.keys(DOC_TYPE_HUES) as AtlasDocType[];
	const count = $derived(activeFilterCount($page.url));
	let open = $state(false);

	function toggleEdgeKind(k: EdgeKind) {
		const next = filters.edgeKinds.includes(k)
			? filters.edgeKinds.filter((x) => x !== k)
			: [...filters.edgeKinds, k];
		goto(buildFiltersUrl($page.url, { edgeKinds: next }), { replaceState: true });
	}
	function toggleDocType(dt: AtlasDocType) {
		const next = filters.docTypes.includes(dt)
			? filters.docTypes.filter((x) => x !== dt)
			: [...filters.docTypes, dt];
		goto(buildFiltersUrl($page.url, { docTypes: next }), { replaceState: true });
	}
	function setLens(raw: string) {
		goto(buildFiltersUrl($page.url, { lensId: raw.trim() || null }), { replaceState: true });
	}
</script>

<div class="filter-popover">
	<button type="button" class="trigger" class:active={count > 0} onclick={() => (open = !open)}>
		⚑ Filters{#if count > 0}<span class="badge">{count}</span>{/if}
	</button>
	{#if open}
		<div class="panel">
			<div class="filter-group">
				<span class="filter-label">EDGE</span>
				{#each EDGE_KIND_OPTIONS as k (k)}
					<button type="button" class="chip" class:active={filters.edgeKinds.includes(k)} onclick={() => toggleEdgeKind(k)}>{k}</button>
				{/each}
			</div>
			<div class="filter-group">
				<span class="filter-label">TYPE</span>
				{#each DOC_TYPE_OPTIONS as dt (dt)}
					<button type="button" class="chip" class:active={filters.docTypes.includes(dt)} style="--chip-color: {DOC_TYPE_HUES[dt]}" onclick={() => toggleDocType(dt)}>{dt}</button>
				{/each}
			</div>
			<div class="filter-group">
				<span class="filter-label">LENS</span>
				<input class="lens-input" type="text" placeholder="lens id" value={filters.lensId ?? ''} onchange={(e) => setLens((e.currentTarget as HTMLInputElement).value)} />
			</div>
		</div>
	{/if}
</div>

<style>
	.filter-popover { position: relative; }
	.trigger {
		background: #1c1c1c; border: 1px solid #4a5162; border-radius: 6px;
		color: var(--color-quiet-ink, #c9ced9); cursor: pointer; font-size: 12px; padding: 3px 10px;
		display: inline-flex; align-items: center; gap: 6px;
	}
	.trigger.active { border-color: #6e5a2a; color: #e8cf8f; }
	.badge {
		background: #6e5a2a; color: #1a1206; border-radius: 8px; font-size: 10px;
		padding: 0 5px; line-height: 1.5;
	}
	.panel {
		position: absolute; right: 0; top: calc(100% + 6px); z-index: 10;
		background: #14171d; border: 1px solid rgba(255, 255, 255, 0.1); border-radius: 8px;
		padding: 8px 12px; display: flex; flex-direction: column; gap: 8px; min-width: 220px;
	}
	.filter-group { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
	.filter-label { font: 8.5px monospace; letter-spacing: 0.2em; color: #6a727e; margin-right: 2px; }
	.chip {
		background: none; border: 1px solid var(--chip-color, #4a5162); color: var(--color-quiet-ink, #c9ced9);
		border-radius: 10px; padding: 1px 8px; font-size: 10.5px; cursor: pointer; opacity: 0.55;
	}
	.chip.active { opacity: 1; background: color-mix(in srgb, var(--chip-color, #4a5162) 22%, transparent); }
	.lens-input {
		background: rgba(255, 255, 255, 0.04); border: 1px solid #4a5162; border-radius: 6px;
		color: var(--color-quiet-ink, #c9ced9); font-size: 11px; padding: 2px 6px; width: 96px;
	}
</style>
