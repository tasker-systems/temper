<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { LayoutData } from './$types';
	import { page } from '$app/stores';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import CommandPalette from '$lib/components/CommandPalette.svelte';
	import { sidebarCollapsed } from '$lib/stores/sidebar.svelte';

	let { data, children }: { data: LayoutData; children: Snippet } = $props();

	let palette: CommandPalette;

	// Seed collapse from stored preference or the route default on each navigation
	// (explicit user toggles persist and win). $effect re-runs when the path changes.
	$effect(() => {
		sidebarCollapsed.initFor($page.url.pathname);
	});

	function onKeydown(e: KeyboardEvent) {
		if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
			e.preventDefault();
			palette.toggle();
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div class="flex h-screen bg-zinc-950 text-zinc-100">
	<Sidebar
		contexts={data.contexts ?? []}
		user={data.profile
			? { display_name: data.profile.display_name, email: data.profile.email ?? '' }
			: null}
		isAdmin={data.entitlements?.is_admin ?? false}
		instanceName={data.instanceName ?? null}
		collapsed={sidebarCollapsed.value}
		onToggle={() => sidebarCollapsed.toggle()}
	/>
	<!--
		main is a flex-col so page content can size itself against a resolved
		parent height (via `h-full` or `flex-1 min-h-0`) instead of subtracting
		a magic header height with `calc(100vh - 4rem)`. The scroll boundary
		lives on the inner wrapper — pages that want to fill the viewport
		(graph) get a bounded height; pages with long content scroll inside it.
	-->
	<main class="flex flex-1 min-w-0 flex-col">
		<header
			class="flex items-center gap-3 px-6 py-3 bg-zinc-950 border-b border-zinc-800"
		>
			<button
				onclick={() => palette.toggle()}
				class="flex-1 flex items-center justify-between px-3 py-1.5 bg-zinc-900 border border-zinc-800 rounded text-sm text-zinc-500 hover:border-zinc-700"
			>
				<span>Search the vault...</span>
				<kbd
					class="text-[10px] bg-zinc-800 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-500"
					>&#8984;K</kbd
				>
			</button>
		</header>
		<div class="flex-1 min-h-0 overflow-y-auto">
			{@render children()}
		</div>
	</main>
</div>

<CommandPalette bind:this={palette} />
