<script lang="ts">
	import type { PageData } from './$types';
	import ResourceMetaHeader from '$lib/components/ResourceMetaHeader.svelte';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>{data.resource.title} — temper</title>
</svelte:head>

<div class="p-6 max-w-4xl">
	<div class="mb-6">
		<a
			href="/vault/{data.resource.owner_handle}/{data.resource.context_name}"
			class="text-xs font-mono tracking-wide text-zinc-500 hover:text-zinc-300 transition-colors"
		>
			&larr; {data.resource.context_name}
		</a>
	</div>

	<div class="mb-8">
		<ResourceMetaHeader resource={data.resource} />
	</div>

	<MarkdownRenderer markdown={data.content} />
</div>
