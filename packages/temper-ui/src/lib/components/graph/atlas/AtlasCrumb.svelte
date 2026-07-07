<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { Focus } from '$lib/graph/atlas/nav';
	import { buildHomeUrl, buildCogmapUrl, buildAscendUrl } from '$lib/graph/atlas/nav';
	import { crumbModel, type CrumbSegment } from '$lib/graph/atlas/crumbModel';

	interface Props {
		cogmapName: string | null;
		focusPath: Focus[];
		crumbTerritory: { id: string; label: string | null } | null;
		seedTitle: string | null;
		cogmapId: string | null;
		scopeFilter: string | null;
	}
	let { cogmapName, focusPath, crumbTerritory, seedTitle, cogmapId, scopeFilter }: Props = $props();

	const segments = $derived(
		crumbModel({ cogmapName, focusPath, crumbTerritory, seedTitle, scopeFilter })
	);
	const canAscend = $derived(focusPath.length > 0);

	// Navigate to a specific `?focus=` path (drill segment click). PUSH — this is a
	// scope/drill transition (Beat 1 history policy).
	function gotoFocus(focusValue: string) {
		const u = new URL($page.url);
		u.searchParams.set('focus', focusValue);
		goto(`${u.pathname}${u.search}`);
	}

	function onSegment(seg: CrumbSegment) {
		if (seg.kind === 'home') return goto(buildHomeUrl($page.url));
		if (seg.kind === 'cogmap' && cogmapId) return goto(buildCogmapUrl($page.url, cogmapId));
		if (seg.focusPath) return gotoFocus(seg.focusPath);
		// ancestors are a de-emphasized set with no drill target
	}
</script>

<nav class="crumb-bar" aria-label="Atlas breadcrumb">
	<button
		class="ascend"
		type="button"
		disabled={!canAscend}
		title="Up one level"
		aria-label="Up one level"
		onclick={() => goto(buildAscendUrl($page.url))}>↑</button
	>
	{#each segments as seg, i (i)}
		{#if i > 0}<span class="sep">›</span>{/if}
		{#if seg.kind === 'ancestor'}
			<span class="seg ancestor">{seg.label}</span>
		{:else}
			<button
				class="seg {seg.kind} {i === segments.length - 1 ? 'current' : ''}"
				type="button"
				onclick={() => onSegment(seg)}>{seg.label}</button
			>
		{/if}
	{/each}
</nav>

<style>
	.crumb-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
		min-width: 0;
		flex-wrap: wrap;
	}
	.ascend {
		background: none;
		border: 1px solid #4a5162;
		border-radius: 6px;
		color: inherit;
		cursor: pointer;
		padding: 0 7px;
		line-height: 1.6;
	}
	.ascend:disabled {
		opacity: 0.3;
		cursor: default;
	}
	.seg {
		background: none;
		border: 0;
		padding: 2px 4px;
		font: inherit;
		color: inherit;
		cursor: pointer;
		border-radius: 5px;
	}
	.seg:not(.ancestor):hover {
		background: rgba(255, 255, 255, 0.06);
	}
	.seg.current {
		font-weight: 600;
		cursor: default;
	}
	.ancestor {
		opacity: 0.6;
	}
	.sep {
		opacity: 0.4;
	}
</style>
