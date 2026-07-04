<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';

	let { data }: { data: PageData } = $props();

	// M6: keying AtlasCanvas on the scoped view remounts it on re-scope, resetting the camera.
	const viewKey = $derived(
		`${data.teamId ?? 'home'}|${data.focus.kind}:${data.focus.kind === 'none' ? '' : data.focus.id}`
	);
</script>

<div class="atlas-page">
	{#if data.scope}
		<ScopeBar scope={data.scope} />
	{:else}
		<nav class="scope-bar home">Atlas · your teams</nav>
	{/if}
	{#key viewKey}
		<AtlasCanvas
			teamId={data.teamId}
			tier={data.tier}
			focus={data.focus}
			territories={data.territories}
			slice={data.slice}
			neighborhood={data.neighborhood}
			teams={data.teams}
			zones={data.scope?.zones ?? []}
		/>
	{/key}
</div>

<style>
	.atlas-page {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
	.scope-bar.home {
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
</style>
