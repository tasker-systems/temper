<script lang="ts">
	import type { TeamScopeView } from '$lib/types/generated/graph_scope';

	interface Props {
		scope: TeamScopeView;
	}
	let { scope }: Props = $props();
</script>

<nav class="scope-bar">
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
	.crumb.current {
		font-weight: 600;
	}
	.sep {
		opacity: 0.4;
	}
</style>
