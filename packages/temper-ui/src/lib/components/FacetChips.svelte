<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { parseFilters, toggleDocType } from '$lib/vault-filters';

	interface Props {
		facets: Record<string, number> | null;
	}

	let { facets }: Props = $props();

	let activeDocTypes = $derived(parseFilters($page.url).docTypes);

	let sorted = $derived(
		facets
			? Object.entries(facets)
					.sort(([, a], [, b]) => b - a)
					.map(([name, count]) => ({ name, count }))
			: []
	);

	function toggle(name: string) {
		goto(toggleDocType($page.url, name), { replaceState: true });
	}
</script>

{#if sorted.length > 0}
	<div class="flex flex-wrap gap-1.5">
		{#each sorted as { name, count }}
			{@const active = activeDocTypes.includes(name)}
			<button
				class="inline-flex items-center gap-1.5 rounded px-2.5 py-1 text-xs font-mono tracking-wide transition-colors
					{active
					? 'bg-quiet-accent/15 text-quiet-accent border border-quiet-border'
					: 'bg-zinc-800/50 text-zinc-400 border border-zinc-700/50 hover:text-zinc-200 hover:border-zinc-600'}"
				onclick={() => toggle(name)}
				aria-pressed={active}
			>
				{name}
				<span class="text-[10px] {active ? 'text-quiet-accent/70' : 'text-zinc-600'}">{count}</span>
			</button>
		{/each}
	</div>
{/if}
