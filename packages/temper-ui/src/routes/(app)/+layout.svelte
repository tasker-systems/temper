<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { LayoutData } from './$types';
	import { page } from '$app/stores';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import CommandPalette from '$lib/components/CommandPalette.svelte';
	import NavProgress from '$lib/components/NavProgress.svelte';
	import { sidebarCollapsed, sidebarGroups } from '$lib/stores/sidebar.svelte';

	let { data, children }: { data: LayoutData; children: Snippet } = $props();

	let palette: CommandPalette;

	// Seed collapse from stored preference or the route default on each navigation
	// (explicit user toggles persist and win). $effect re-runs when the path changes.
	$effect(() => {
		sidebarCollapsed.initFor($page.url.pathname);
		sidebarGroups.init();
	});

	function onKeydown(e: KeyboardEvent) {
		if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
			e.preventDefault();
			palette.toggle();
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div class="flex h-screen bg-tp-ground text-tp-text">
	<NavProgress />
	<Sidebar
		contexts={data.contexts}
		teams={data.teams}
		selfProfileId={data.profile.id}
		user={data.profile
			? { display_name: data.profile.display_name, email: data.profile.email ?? '' }
			: null}
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
			class="flex items-center gap-3 px-6 py-3 bg-tp-ground border-b border-tp-rule"
		>
			<button
				onclick={() => palette.toggle()}
				class="flex-1 flex items-center justify-between px-3 py-1.5 bg-tp-surface border border-tp-rule-strong rounded text-sm text-tp-text-subtle hover:border-tp-accent-line-soft"
			>
				<span>Search the vault...</span>
				<kbd
					class="text-[10px] bg-tp-surface-raised border border-tp-rule-strong rounded px-1.5 py-0.5 text-tp-text-subtle"
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
