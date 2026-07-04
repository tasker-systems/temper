<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { TeamScopeView } from '$lib/types/generated/graph_scope';
	import { buildHomeUrl } from '$lib/graph/atlas/nav';

	interface Props {
		scope: TeamScopeView;
	}
	let { scope }: Props = $props();
</script>

<nav class="scope-bar">
	<button class="crumb home" type="button" onclick={() => goto(buildHomeUrl($page.url), { replaceState: true })}>⌂ Atlas</button>
	<span class="sep">/</span>
	{#each scope.ancestors as ancestor (ancestor.id)}
		<span class="crumb">{ancestor.name}</span>
		<span class="sep">/</span>
	{/each}
	<span class="crumb current">{scope.team.name}</span>
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
