<script lang="ts">
	import type { GraphNode } from '$lib/types/generated/graph';

	interface Props {
		node: GraphNode | null;
		x: number;
		y: number;
	}

	let { node, x, y }: Props = $props();
</script>

{#if node}
	<div
		class="pointer-events-none absolute z-50 min-w-40 max-w-64 rounded border border-neutral-300 bg-white/95 px-3 py-2 text-xs shadow-md dark:border-neutral-700 dark:bg-neutral-900/95"
		style="left: {x + 12}px; top: {y + 12}px;"
	>
		<div class="mb-1 flex items-center gap-2">
			<span
				class="rounded-sm bg-neutral-100 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-neutral-600 dark:bg-neutral-800 dark:text-neutral-400"
				>{node.doc_type}</span
			>
			<span class="text-neutral-500">{node.edge_count} edges</span>
		</div>
		<div class="font-medium leading-snug">{node.title}</div>
	</div>
{/if}
