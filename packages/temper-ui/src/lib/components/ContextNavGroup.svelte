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

	// Collapsing must never cost the reader their location — but forcing the group
	// back open would make a control labelled "Collapse" do nothing, which is the
	// affordance overstating itself. So the preference stays authoritative and the
	// heading carries the active mark instead: where you are stays legible at the
	// group level, and the disclosure says exactly what it does.
	let holdsActive = $derived(group.contexts.some((c) => isActive(c)));
	let listId = $derived(`nav-group-${group.key}`);
</script>

<button
	type="button"
	onclick={onToggle}
	aria-expanded={!collapsed}
	aria-controls={listId}
	class="flex w-full items-center gap-1.5 px-3 pt-4 pb-1 text-left text-xs
	       {collapsed && holdsActive ? 'text-zinc-100' : 'text-zinc-300'} hover:text-zinc-100"
	title={collapsed ? `Expand ${group.label}` : `Collapse ${group.label}`}
>
	<span class="w-2.5 flex-shrink-0 text-[10px] leading-none text-zinc-500" aria-hidden="true"
		>{collapsed ? '▸' : '▾'}</span
	>
	<span class="flex-1 truncate">{group.label}</span>
	{#if collapsed && holdsActive}
		<span class="w-1.5 h-1.5 rounded-sm bg-quiet-accent" title="You are in this group"></span>
	{/if}
	<!-- Same unit and position as a place's count, so "how much work is in here"
	     reads the same at the group level as inside it. -->
	<span class="text-xs text-zinc-600">{group.resourceCount}</span>
</button>

<div id={listId} hidden={collapsed}>
	{#each group.contexts as ctx (ctx.id)}
		<a
			href={contextHref(ctx.owner_ref, ctx.slug)}
			class="flex items-center gap-2 py-1.5 pl-6 pr-3 text-sm transition-colors
			       {isActive(ctx)
				? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(1.5rem-2px)]'
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
				class="flex items-center gap-2 pl-11 pr-3 py-1.5 text-sm transition-colors
				       {isGraphActive(ctx)
					? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(2.75rem-2px)]'
					: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
			>
				<span
					class="w-1.5 h-1.5 rounded-sm {isGraphActive(ctx) ? 'bg-quiet-accent' : 'bg-zinc-600'}"
				></span>
				<span class="flex-1 truncate">Graph</span>
			</a>
		{/if}
	{:else}
		<div class="py-1.5 pl-6 pr-3 text-sm text-zinc-600">No contexts</div>
	{/each}
</div>
