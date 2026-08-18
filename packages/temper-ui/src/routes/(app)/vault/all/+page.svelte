<script lang="ts">
	import VaultBrowser from '$lib/components/vault/VaultBrowser.svelte';
	import type { PageData } from './$types';

	// `contexts` comes from the `(app)` layout load, which already fetches it for the sidebar.
	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>Vault — temper</title>
</svelte:head>

<VaultBrowser title="All resources" list={data.list} contexts={data.contexts} />
