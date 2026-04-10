<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { LayoutData } from './$types';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import CommandPalette from '$lib/components/CommandPalette.svelte';

	let { data, children }: { data: LayoutData; children: Snippet } = $props();

	let palette: CommandPalette;

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
	/>
	<main class="flex-1 overflow-y-auto">
		<header
			class="sticky top-0 z-10 flex items-center gap-3 px-6 py-3 bg-zinc-950 border-b border-zinc-800"
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
		{@render children()}
	</main>
</div>

<CommandPalette bind:this={palette} />
