<script lang="ts">
	import type { Snippet } from 'svelte';
	import Nav from '$lib/components/landing/Nav.svelte';
	import { onMount } from 'svelte';
	import '../../marketing.css';

	let { children }: { children: Snippet } = $props();

	// Arriving by client navigation from a light app page, <html> is still light: the wrapper is
	// dark, but the root's scrollbar and overscroll would show the light ground. Marketing holds the
	// root dark while it is mounted and hands it back on leaving; the (app) layout re-applies the
	// reader's theme in any case. Served directly, the root is already dark (hooks.server.ts).
	onMount(() => {
		const root = document.documentElement;
		const was = root.getAttribute('data-theme');
		root.setAttribute('data-theme', 'quiet-instrument');
		return () => {
			if (was) root.setAttribute('data-theme', was);
		};
	});
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
