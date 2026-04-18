<script lang="ts">
	import type { PageData } from './$types';
	import GraphCanvas from '$lib/components/graph/GraphCanvas.svelte';
	import EmptyState from '$lib/components/EmptyState.svelte';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>Knowledge graph — {data.context}</title>
</svelte:head>

<div class="mx-auto max-w-7xl p-6">
	<header class="mb-6">
		<h1 class="text-2xl font-semibold tracking-tight">Knowledge graph</h1>
		<p class="text-sm text-neutral-500">
			{data.context} — {data.subgraph.nodes.length} nodes, {data.subgraph.edges.length} edges
		</p>
	</header>

	{#if data.subgraph.nodes.length === 0}
		<EmptyState
			message="No concepts yet. Run `temper graph index` to generate concepts from this context, then reload."
		/>
	{:else}
		<GraphCanvas
			nodes={data.subgraph.nodes}
			edges={data.subgraph.edges}
			owner={data.owner}
			context={data.context}
		/>
	{/if}
</div>
