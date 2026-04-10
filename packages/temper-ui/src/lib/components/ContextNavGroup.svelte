<script lang="ts">
	import { page } from '$app/stores';
	import type { ContextRowWithCounts } from '$lib/types';

	interface Props {
		label: string;
		ownerPrefix: string;
		contexts: ContextRowWithCounts[];
	}

	let { label, ownerPrefix, contexts }: Props = $props();

	function isActive(ctx: ContextRowWithCounts): boolean {
		return $page.params.owner === ownerPrefix && $page.params.context === ctx.name;
	}
</script>

<div class="px-3 pt-4 pb-1 text-[10px] uppercase tracking-widest text-zinc-500">
	{label}
</div>
{#each contexts as ctx}
	<a
		href="/vault/{ownerPrefix}/{ctx.name}"
		class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
		       {isActive(ctx)
			? 'border-l-2 border-yellow-500 bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
			: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
	>
		<span
			class="w-1.5 h-1.5 rounded-sm {isActive(ctx) ? 'bg-yellow-500' : 'bg-zinc-600'}"
		></span>
		<span class="flex-1 truncate">{ctx.name}</span>
		<span class="text-xs text-zinc-600">{ctx.resource_count}</span>
	</a>
{/each}
