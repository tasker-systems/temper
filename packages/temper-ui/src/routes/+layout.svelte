<script lang="ts">
	import type { Snippet } from 'svelte';
	import '../app.css';
	// Self-hosted variable fonts — no third-party font CDN, so no IP/User-Agent
	// egress to Google on every page load (and no `fonts.googleapis.com` in CSP).
	// `standard` carries the opsz+wght axes the old Google Fonts URL requested.
	import '@fontsource-variable/jetbrains-mono';
	import '@fontsource-variable/source-serif-4/standard.css';
	import '@fontsource-variable/source-serif-4/standard-italic.css';

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
		background: var(--obsidian);
		color: var(--parchment);
		font-family: var(--font-serif);
		line-height: 1.7;
		-webkit-font-smoothing: antialiased;
	}

	:global(body) {
		background: var(--obsidian);
	}
</style>
