<script lang="ts">
	import type { PageData } from './$types';
	import AtlasCanvas from '$lib/components/graph/atlas/AtlasCanvas.svelte';
	import ScopeBar from '$lib/components/graph/atlas/ScopeBar.svelte';

	let { data }: { data: PageData } = $props();
</script>

<div class="atlas-page">
	<ScopeBar scope={data.scope} />
	<AtlasCanvas tier={data.tier} territories={data.territories} zones={data.scope.zones} />
</div>

<style>
	.atlas-page {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
</style>
