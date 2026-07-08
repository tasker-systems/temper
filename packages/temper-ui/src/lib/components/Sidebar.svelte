<script lang="ts">
	import { page } from '$app/stores';
	import ContextNavGroup from './ContextNavGroup.svelte';
	import type { ContextRowWithCounts } from '$lib/types';

	interface Props {
		contexts: ContextRowWithCounts[];
		user: { display_name: string; email: string } | null;
		isAdmin: boolean;
		/** Operator-configured instance brand ("temper @ acme"); null → default. */
		instanceName?: string | null;
		collapsed: boolean;
		onToggle: () => void;
	}

	let { contexts, user, isAdmin, instanceName = null, collapsed, onToggle }: Props = $props();

	let brand = $derived(instanceName?.trim() || 'temper');

	let myContexts = $derived(contexts.filter((c) => c.kb_owner_table === 'kb_profiles'));
	let teamContexts = $derived(contexts.filter((c) => c.kb_owner_table === 'kb_teams'));

	let pathname = $derived($page.url.pathname as string);
	let isAllActive = $derived(pathname === '/vault/all' || pathname === '/vault');
</script>

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
			class="block px-3 py-2 text-sm text-zinc-400 hover:text-zinc-200 text-center"
			title="Home"
			aria-label="Home"
		>
			⌂
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

			{#if myContexts.length > 0}
				<ContextNavGroup label="Contexts" contexts={myContexts} />
			{/if}

			{#if teamContexts.length > 0}
				<ContextNavGroup label="Teams" contexts={teamContexts} />
			{/if}
		</nav>

		<div class="border-t border-zinc-800 py-2">
			<a
				href="/teams"
				class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200"
			>
				<span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Teams
			</a>
			{#if isAdmin}
				<a
					href="/admin/access"
					class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200"
				>
					<span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Admin
				</a>
			{/if}
			<a
				href="/settings"
				class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200"
			>
				<span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Settings
			</a>
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
