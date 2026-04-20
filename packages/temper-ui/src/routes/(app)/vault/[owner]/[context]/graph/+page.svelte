<script lang="ts">
	import type { PageData } from './$types';
	import KnowledgeGraph from '$lib/components/graph/KnowledgeGraph.svelte';
	import EmptyState from '$lib/components/EmptyState.svelte';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>Knowledge graph — {data.context}</title>
</svelte:head>

<div class="flex h-[calc(100vh-4rem)] flex-col">
	<header class="border-b border-neutral-800 bg-neutral-950 px-6 py-3">
		<h1 class="text-sm font-medium tracking-wide text-neutral-200">Knowledge graph</h1>
		<p class="text-xs text-neutral-500">
			{data.context} — {data.subgraph.nodes.length} nodes, {data.subgraph.edges.length} edges
		</p>
	</header>

	{#if data.subgraph.nodes.length === 0}
		<div class="p-6">
			<EmptyState
				message="No concepts yet. Run `temper graph index` to generate concepts from this context, then reload."
			/>
		</div>
	{:else}
		<div class="min-h-0 flex-1">
			<KnowledgeGraph
				nodes={data.subgraph.nodes}
				edges={data.subgraph.edges}
				owner={data.owner}
				context={data.context}
			/>
		</div>
	{/if}
</div>
