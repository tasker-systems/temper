<script lang="ts">
	import type { PageData } from './$types';
	import RuleHeading from '$lib/components/RuleHeading.svelte';
	import FacetChips from '$lib/components/FacetChips.svelte';
	import VaultGrid from '$lib/components/VaultGrid.svelte';
	import { columnsFor } from '$lib/vault-columns';

	let { data }: { data: PageData } = $props();

	// Interim: kind-scoped column narrowing (revealedKind) is Task 7's job, which also mounts
	// FilterBar here. Until then this page shows the mixed-kind column set unconditionally.
	const columns = columnsFor(null);
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

	<VaultGrid
		rows={data.rows}
		{columns}
		total={data.total}
		returned={data.returned}
		truncated={data.truncated}
		limit={data.limit}
		offset={data.offset}
	/>
</div>
