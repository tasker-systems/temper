<script lang="ts">
	import GraphPage from '$lib/components/graph/GraphPage.svelte';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
</script>

<GraphPage {data} />
