<script lang="ts">
	import VaultBrowser from '$lib/components/vault/VaultBrowser.svelte';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>{data.context} — temper</title>
</svelte:head>

<VaultBrowser
	title={data.context}
	list={data.list}
	contexts={data.contexts}
	fixedContext
	captionPrefix={data.owner}
/>
