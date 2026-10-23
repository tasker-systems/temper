<script lang="ts">
	import type { Snippet } from 'svelte';
	import Nav from '$lib/components/landing/Nav.svelte';
	import '../../marketing.css';

	let { children }: { children: Snippet } = $props();
</script>

<svelte:head>
	<title>temper — clarify your intention</title>
	<meta
		name="description"
		content="A knowledge base for builders. Know what was decided, what's deferred, and what comes next. Every session builds on the last. Temper your context."
	/>
</svelte:head>

<!-- Marketing is dark-only and paints from its own names (marketing.css): the wrapper holds both,
	whatever theme the reader's <html> is in. -->
<div class="marketing" data-theme="quiet-instrument">
	<Nav />
	<main>
		{@render children()}
	</main>
</div>
