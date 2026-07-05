<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { buildHomeUrl } from '$lib/graph/atlas/nav';

	// Cogmap scope (entered via ?cogmap=) has no team ScopeBar, so it needs its own
	// breadcrumb (B2) — without it the dock showed plain "Atlas · your teams" text
	// with no way back. ⌂ Atlas PUSHes history so Back returns into the cogmap.
	interface Props {
		name: string;
	}
	let { name }: Props = $props();
</script>

<nav class="scope-bar">
	<button class="crumb home" type="button" onclick={() => goto(buildHomeUrl($page.url))}>⌂ Atlas</button>
	<span class="sep">/</span>
	<span class="crumb current">{name}</span>
</nav>

<style>
	.scope-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 8px 14px;
		font-size: 13px;
		color: var(--color-quiet-ink, #c9ced9);
	}
	.crumb.home {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: inherit;
		cursor: pointer;
	}
	.crumb.current {
		font-weight: 600;
	}
	.sep {
		opacity: 0.4;
	}
</style>
