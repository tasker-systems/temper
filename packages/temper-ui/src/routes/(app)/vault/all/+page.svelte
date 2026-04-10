<script lang="ts">
	import type { PageData } from './$types';
	import RuleHeading from '$lib/components/RuleHeading.svelte';
	import FacetChips from '$lib/components/FacetChips.svelte';
	import VaultGrid from '$lib/components/VaultGrid.svelte';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>Vault — temper</title>
</svelte:head>

<div class="p-6">
	<div class="mb-6">
		<RuleHeading title="All resources" caption="{data.total} total" />
	</div>

	<div class="mb-4">
		<FacetChips facets={data.facets} />
	</div>

	<VaultGrid rows={data.rows} total={data.total} />
</div>
