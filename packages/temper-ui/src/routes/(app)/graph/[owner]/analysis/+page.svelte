<script lang="ts">
	import AnalysisPage from '$lib/components/graph/AnalysisPage.svelte';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
</script>

<svelte:head>
	<title>{data.place ? `${data.place.title} — measurements` : 'Measurements'} · Temper</title>
</svelte:head>

<AnalysisPage {data} />
