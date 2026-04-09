<script lang="ts">
	import type { Snippet } from 'svelte';
	import '../app.css';

	let { children }: { children: Snippet } = $props();
</script>

<div class="app-shell">
	{@render children()}
</div>

<style>
	/* Global app shell — dark "Quiet Instrument" base inherited by every
	   route group. The (public) marketing pages keep this look directly;
	   the authed (app) pages render lighter cards on top of it. */
	.app-shell {
		min-height: 100vh;
		background: var(--bg);
		color: var(--text);
		font-family: var(--serif);
		line-height: 1.7;
		-webkit-font-smoothing: antialiased;
	}

	:global(body) {
		background: var(--bg);
	}
</style>
