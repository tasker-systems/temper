<script lang="ts">
	import VaultBrowser from '$lib/components/vault/VaultBrowser.svelte';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>Search — temper</title>
</svelte:head>

<VaultBrowser
	title="Search: {data.query}"
	list={data.list}
	loadError={data.loadError}
	contexts={data.contexts}
/>
