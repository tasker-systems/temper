<script lang="ts">
	import { page } from '$app/stores';
	import {
		contextHref,
		contextGraphHref,
		isContextLocation,
		isContextGraphLocation
	} from '$lib/vault-url';
	import type { NavGroup } from '$lib/nav-groups';
	import type { ContextRowWithCounts } from '$lib/types';

	interface Props {
		group: NavGroup;
		/** Persisted per-group preference — see `stores/sidebar.svelte.ts`. */
		collapsed: boolean;
		onToggle: () => void;
	}

	let { group, collapsed, onToggle }: Props = $props();

	function isActive(ctx: ContextRowWithCounts): boolean {
		return isContextLocation($page.params, $page.url, ctx.owner_ref, ctx.slug);
	}

	function isGraphActive(ctx: ContextRowWithCounts): boolean {
		return isContextGraphLocation($page.params, $page.url, ctx.owner_ref, ctx.slug);
	}

	// A collapsed group must never hide where the reader currently is: the group
	// heading would then be the only thing lit, and the active place invisible.
	let holdsActive = $derived(group.contexts.some((c) => isActive(c)));
	let expanded = $derived(!collapsed || holdsActive);
</script>

<button
	type="button"
	onclick={onToggle}
	aria-expanded={expanded}
	class="flex w-full items-center gap-1.5 px-3 pt-4 pb-1 text-left text-[10px] uppercase
	       tracking-widest text-zinc-500 hover:text-zinc-300"
	title={collapsed ? `Expand ${group.label}` : `Collapse ${group.label}`}
>
	<span class="w-2 flex-shrink-0 text-[8px] leading-none" aria-hidden="true"
		>{expanded ? '▾' : '▸'}</span
	>
	<span class="flex-1 truncate normal-case tracking-normal text-xs text-zinc-400"
		>{group.label}</span
	>
	<!-- Same unit and position as a place's count, so "how much work is in here"
	     reads the same at the group level as inside it. -->
	<span class="text-xs text-zinc-600">{group.resourceCount}</span>
</button>

{#if expanded}
	{#each group.contexts as ctx (ctx.id)}
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
	{:else}
		<div class="px-3 py-1.5 pl-[1.4rem] text-sm text-zinc-600">No contexts</div>
	{/each}
{/if}
