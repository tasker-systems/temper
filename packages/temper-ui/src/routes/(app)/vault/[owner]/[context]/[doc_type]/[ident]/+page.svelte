<script lang="ts">
	import type { PageData } from './$types';
	import ResourceMetaHeader from '$lib/components/ResourceMetaHeader.svelte';
	import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
	import { contextHref } from '$lib/vault-url';

	let { data }: { data: PageData } = $props();

	// context_* are null only for a cogmap-homed resource; this context-shaped
	// route is reached for context-homed resources, but guard defensively.
	let backHref = $derived(
		data.resource.context_owner_ref && data.resource.context_slug
			? contextHref(data.resource.context_owner_ref, data.resource.context_slug)
			: null
	);
</script>

<svelte:head>
	<title>{data.resource.title} — temper</title>
</svelte:head>

<div class="p-6 max-w-4xl">
	{#if backHref}
		<div class="mb-6">
			<a
				href={backHref}
				class="text-xs font-mono tracking-wide text-zinc-500 hover:text-zinc-300 transition-colors"
			>
				&larr; {data.resource.context_name ?? data.resource.context_slug}
			</a>
		</div>
	{/if}

	<div class="mb-8">
		<ResourceMetaHeader resource={data.resource} />
	</div>

	<MarkdownRenderer markdown={data.content} />
</div>
