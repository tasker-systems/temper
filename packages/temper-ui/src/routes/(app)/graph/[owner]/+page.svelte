<script lang="ts">
	import type { PageData } from './$types';
	import AtlasPage from '$lib/components/graph/atlas/AtlasPage.svelte';

	let { data }: { data: PageData } = $props();
</script>

<AtlasPage {data} />
