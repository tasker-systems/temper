<script lang="ts">
	import { page } from '$app/stores';
	import { contextHref, contextGraphHref } from '$lib/vault-url';
	import type { ContextRowWithCounts } from '$lib/types';

	interface Props {
		label: string;
		contexts: ContextRowWithCounts[];
	}

	let { label, contexts }: Props = $props();

	function isActive(ctx: ContextRowWithCounts): boolean {
		return $page.params.owner === ctx.owner_ref && $page.params.context === ctx.slug;
	}

	function isGraphActive(ctx: ContextRowWithCounts): boolean {
		return isActive(ctx) && $page.url.pathname.endsWith('/graph');
	}
</script>

<div class="px-3 pt-4 pb-1 text-[10px] uppercase tracking-widest text-zinc-500">
	{label}
</div>
{#each contexts as ctx}
	<a
		href={contextHref(ctx.owner_ref, ctx.slug)}
		class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
		       {isActive(ctx)
			? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
			: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
	>
		<span
			class="w-1.5 h-1.5 rounded-sm {isActive(ctx) ? 'bg-quiet-accent' : 'bg-zinc-600'}"
		></span>
		<span class="flex-1 truncate">{ctx.name}</span>
		<span class="text-xs text-zinc-600">{ctx.resource_count}</span>
	</a>
	{#if isActive(ctx)}
		<a
			href={contextGraphHref(ctx.owner_ref, ctx.slug)}
			class="flex items-center gap-2 pl-8 pr-3 py-1.5 text-sm transition-colors
			       {isGraphActive(ctx)
				? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(2rem-2px)]'
				: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
		>
			<span
				class="w-1.5 h-1.5 rounded-sm {isGraphActive(ctx) ? 'bg-quiet-accent' : 'bg-zinc-600'}"
			></span>
			<span class="flex-1 truncate">Graph</span>
		</a>
	{/if}
{/each}
