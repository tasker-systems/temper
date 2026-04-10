<script lang="ts">
	import type { Snippet } from 'svelte';
	import Nav from '$lib/components/landing/Nav.svelte';

	let { children }: { children: Snippet } = $props();
</script>

<svelte:head>
	<title>temper — clarify your intention</title>
	<meta
		name="description"
		content="A knowledge base for builders. Know what was decided, what's deferred, and what comes next. Every session builds on the last. Temper your context."
	/>
</svelte:head>

<Nav />
<main>
	{@render children()}
</main>
