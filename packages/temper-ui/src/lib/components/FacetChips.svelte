<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';

	interface Props {
		facets: Record<string, number> | null;
	}

	let { facets }: Props = $props();

	let activeFilter = $derived($page.url.searchParams.get('doc_type_name'));

	let sorted = $derived(
		facets
			? Object.entries(facets)
					.sort(([, a], [, b]) => b - a)
					.map(([name, count]) => ({ name, count }))
			: []
	);

	function toggle(name: string) {
		const url = new URL($page.url);
		if (activeFilter === name) {
			url.searchParams.delete('doc_type_name');
		} else {
			url.searchParams.set('doc_type_name', name);
		}
		url.searchParams.delete('offset');
		goto(url.toString(), { replaceState: true });
	}
</script>

{#if sorted.length > 0}
	<div class="flex flex-wrap gap-1.5">
		{#each sorted as { name, count }}
			<button
				class="inline-flex items-center gap-1.5 rounded px-2.5 py-1 text-xs font-mono tracking-wide transition-colors
					{activeFilter === name
					? 'bg-quiet-accent/15 text-quiet-accent border border-quiet-border'
					: 'bg-zinc-800/50 text-zinc-400 border border-zinc-700/50 hover:text-zinc-200 hover:border-zinc-600'}"
				onclick={() => toggle(name)}
			>
				{name}
				<span class="text-[10px] {activeFilter === name ? 'text-quiet-accent/70' : 'text-zinc-600'}"
					>{count}</span
				>
			</button>
		{/each}
	</div>
{/if}
