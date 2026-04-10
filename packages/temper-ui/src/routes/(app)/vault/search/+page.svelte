<script lang="ts">
	import RuleHeading from '$lib/components/RuleHeading.svelte';
	import FacetChips from '$lib/components/FacetChips.svelte';
	import VaultGrid from '$lib/components/VaultGrid.svelte';
	import EmptyState from '$lib/components/EmptyState.svelte';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
</script>

<div class="flex flex-col gap-4 p-6 h-full">
	<div class="flex items-center gap-3">
		<RuleHeading title="Search: {data.query}" caption="{data.total ?? 0} results" />
		<a href="/vault/all" class="text-xs text-zinc-500 hover:text-zinc-300 ml-auto"
			>&times; Clear</a
		>
	</div>

	<FacetChips facets={data.facets ?? null} />

	{#if data.rows?.length > 0}
		<VaultGrid rows={data.rows} total={data.total} limit={data.limit} offset={data.offset} />
	{:else}
		<EmptyState
			message='No results for &ldquo;{data.query}&rdquo;'
			action={{ label: 'Browse all', href: '/vault/all' }}
		/>
	{/if}
</div>
