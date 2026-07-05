<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TeamScopeView } from '$lib/types/generated/graph_scope';
	import type { EdgeKind } from '$lib/types/generated/graph';
	import { buildHomeUrl, buildFiltersUrl, type GraphFilters } from '$lib/graph/atlas/nav';
	import { DOC_TYPE_HUES, type AtlasDocType } from '$lib/graph/atlas/palette';

	interface Props {
		scope: TeamScopeView;
		filters: GraphFilters;
	}
	let { scope, filters }: Props = $props();

	// The real edge-kind enum accepted by `SliceRequest.edge_kinds` (R4 traversal
	// filter) — distinct from `EDGE_COLORS` in palette.ts, which is a *style*
	// grouping (structural/contradicts/derived) keyed by edge label, not edge_kind.
	const EDGE_KIND_OPTIONS: EdgeKind[] = ['contains', 'leads_to', 'express', 'near'];
	const DOC_TYPE_OPTIONS = Object.keys(DOC_TYPE_HUES) as AtlasDocType[];

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
		const lensId = raw.trim() || null;
		goto(buildFiltersUrl($page.url, { lensId }), { replaceState: true });
	}
</script>

<nav class="scope-bar">
	<button class="crumb home" type="button" onclick={() => goto(buildHomeUrl($page.url), { replaceState: true })}>⌂ Atlas</button>
	<span class="sep">/</span>
	{#each scope.ancestors as ancestor (ancestor.id)}
		<span class="crumb">{ancestor.name}</span>
		<span class="sep">/</span>
	{/each}
	<span class="crumb current">{scope.team.name}</span>
</nav>

<div class="filters">
	<div class="filter-group">
		<span class="filter-label">EDGE</span>
		{#each EDGE_KIND_OPTIONS as k (k)}
			<button
				type="button"
				class="chip"
				class:active={filters.edgeKinds.includes(k)}
				onclick={() => toggleEdgeKind(k)}
			>
				{k}
			</button>
		{/each}
	</div>
	<div class="filter-group">
		<span class="filter-label">TYPE</span>
		{#each DOC_TYPE_OPTIONS as dt (dt)}
			<button
				type="button"
				class="chip"
				class:active={filters.docTypes.includes(dt)}
				style="--chip-color: {DOC_TYPE_HUES[dt]}"
				onclick={() => toggleDocType(dt)}
			>
				{dt}
			</button>
		{/each}
	</div>
	<div class="filter-group">
		<span class="filter-label">LENS</span>
		<input
			class="lens-input"
			type="text"
			placeholder="lens id"
			value={filters.lensId ?? ''}
			onchange={(e) => setLens((e.currentTarget as HTMLInputElement).value)}
		/>
	</div>
</div>

<style>
	.scope-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
	.crumb.home {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: inherit;
		cursor: pointer;
	}
	.crumb.current {
		font-weight: 600;
	}
	.sep {
		opacity: 0.4;
	}
	.filters {
		padding: 4px 14px 10px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.filter-group {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px;
	}
	.filter-label {
		font: 8.5px monospace;
		letter-spacing: 0.2em;
		color: #6a727e;
		margin-right: 2px;
	}
	.chip {
		background: none;
		border: 1px solid var(--chip-color, #4a5162);
		color: var(--color-quiet-ink, #c9ced9);
		border-radius: 10px;
		padding: 1px 8px;
		font-size: 10.5px;
		cursor: pointer;
		opacity: 0.55;
	}
	.chip.active {
		opacity: 1;
		background: color-mix(in srgb, var(--chip-color, #4a5162) 22%, transparent);
	}
	.lens-input {
		background: rgba(255, 255, 255, 0.04);
		border: 1px solid #4a5162;
		border-radius: 6px;
		color: var(--color-quiet-ink, #c9ced9);
		font-size: 11px;
		padding: 2px 6px;
		width: 96px;
	}
</style>
