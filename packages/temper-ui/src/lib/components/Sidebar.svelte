<script lang="ts">
	import { page } from '$app/stores';
	import ContextNavGroup from './ContextNavGroup.svelte';
	import type { ContextRowWithCounts } from '$lib/types';

	interface Props {
		contexts: ContextRowWithCounts[];
		user: { display_name: string; email: string } | null;
		/** Operator-configured instance brand ("temper @ acme"); null → default. */
		instanceName?: string | null;
		collapsed: boolean;
		onToggle: () => void;
	}

	let { contexts, user, instanceName = null, collapsed, onToggle }: Props = $props();

	let brand = $derived(instanceName?.trim() || 'temper');

	let myContexts = $derived(contexts.filter((c) => c.kb_owner_table === 'kb_profiles'));
	let teamContexts = $derived(contexts.filter((c) => c.kb_owner_table === 'kb_teams'));

	let pathname = $derived($page.url.pathname as string);
	let isAllActive = $derived(pathname === '/vault/all' || pathname === '/vault');
	// The whole-vault graph home (`/graph/@me`). A context-scoped graph
	// (`/graph/[owner]?context=<slug>`) belongs to its context's own Graph
	// sub-link, so exclude it here to keep exactly one nav item lit.
	let isGraphActive = $derived(
		pathname.startsWith('/graph/') && !$page.url.searchParams.get('context')
	);
</script>

{#snippet graphGlyph()}
	<!-- Three connected nodes — a minimal graph mark matching the ⌂ Home glyph's weight. -->
	<svg
		width="15"
		height="15"
		viewBox="0 0 16 16"
		fill="none"
		stroke="currentColor"
		stroke-width="1.3"
		stroke-linecap="round"
		stroke-linejoin="round"
		aria-hidden="true"
	>
		<path d="M4.6 6.1 7 3.6M4.4 8.9 7 12M9.2 4.4l1.6 1.4M9.1 11.4l1.7-3.6" />
		<circle cx="3.2" cy="8" r="1.7" fill="currentColor" stroke="none" />
		<circle cx="8" cy="2.7" r="1.7" fill="currentColor" stroke="none" />
		<circle cx="12.4" cy="6.6" r="1.7" fill="currentColor" stroke="none" />
		<circle cx="8" cy="12.9" r="1.7" fill="currentColor" stroke="none" />
	</svg>
{/snippet}

<aside
	class="flex flex-col {collapsed
		? 'w-12'
		: 'w-52'} bg-zinc-900/50 border-r border-zinc-800 overflow-hidden transition-[width] duration-150"
>
	<button
		type="button"
		onclick={onToggle}
		class="block px-3 pt-3 pb-2 border-b border-zinc-800 font-mono text-xs tracking-[0.15em] text-zinc-300 hover:text-zinc-100 text-left truncate"
		title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
	>
		{collapsed ? '≡' : brand}
	</button>
	{#if collapsed}
		<a
			href="/vault/all"
			class="block px-3 py-2 text-sm {isAllActive
				? 'text-zinc-100'
				: 'text-zinc-400 hover:text-zinc-200'} text-center"
			title="All resources"
			aria-label="All resources"
		>
			⌂
		</a>
		<a
			href="/graph/@me"
			class="flex justify-center px-3 py-2 {isGraphActive
				? 'text-zinc-100'
				: 'text-zinc-400 hover:text-zinc-200'}"
			title="Graph"
			aria-label="Graph"
		>
			{@render graphGlyph()}
		</a>
	{/if}
	{#if !collapsed}
		<nav class="flex-1 overflow-y-auto py-2">
			<div class="px-3 pt-2 pb-1 text-[10px] uppercase tracking-widest text-zinc-500">
				Vault
			</div>
			<a
				href="/vault/all"
				class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
				       {isAllActive
					? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
					: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
			>
				<span class="w-1.5 h-1.5 rounded-sm {isAllActive ? 'bg-quiet-accent' : 'bg-zinc-600'}"
				></span>
				All resources
			</a>
			<a
				href="/graph/@me"
				class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
				       {isGraphActive
					? 'border-l-2 border-quiet-accent bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
					: 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
			>
				<span class="w-1.5 h-1.5 rounded-sm {isGraphActive ? 'bg-quiet-accent' : 'bg-zinc-600'}"
				></span>
				Graph
			</a>

			{#if myContexts.length > 0}
				<ContextNavGroup label="Contexts" contexts={myContexts} />
			{/if}

			{#if teamContexts.length > 0}
				<ContextNavGroup label="Teams" contexts={teamContexts} />
			{/if}
		</nav>

		<div class="border-t border-zinc-800 py-2">
			<a
				href="/auth/logout"
				class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200"
			>
				<span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Sign out
			</a>
			{#if user}
				<div class="flex items-center gap-2 px-3 py-2 text-xs text-zinc-500">
					<div class="w-5 h-5 rounded-full bg-zinc-700 flex-shrink-0"></div>
					{user.display_name}
				</div>
			{/if}
		</div>
	{/if}
</aside>
