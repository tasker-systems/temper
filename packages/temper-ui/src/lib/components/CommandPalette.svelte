<script lang="ts">
	import { goto } from '$app/navigation';
	import type { ResourceView } from '$lib/types';
	import { resourceHref, searchHref } from '$lib/vault-url';

	let open = $state(false);
	let query = $state('');
	let results = $state<ResourceView[]>([]);
	let total = $state(0);
	let focused = $state(0);
	let loading = $state(false);
	let failed = $state(false);
	let debounceTimer: ReturnType<typeof setTimeout>;

	export function toggle() {
		open = !open;
		if (open) {
			query = '';
			results = [];
			total = 0;
			focused = 0;
			failed = false;
		}
	}

	async function search(q: string) {
		if (!q.trim()) {
			results = [];
			total = 0;
			failed = false;
			return;
		}
		loading = true;
		try {
			const resp = await fetch(`/_internal/search?q=${encodeURIComponent(q)}`);
			if (!resp.ok) {
				failed = true;
				results = [];
				total = 0;
			} else {
				const data = await resp.json();
				failed = false;
				results = data.rows;
				total = data.total;
			}
		} catch {
			failed = true;
			results = [];
			total = 0;
		}
		loading = false;
	}

	function onInput() {
		clearTimeout(debounceTimer);
		debounceTimer = setTimeout(() => search(query), 150);
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			open = false;
		} else if (e.key === 'ArrowDown') {
			e.preventDefault();
			focused = Math.min(focused + 1, results.length);
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			focused = Math.max(focused - 1, 0);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			if (focused < results.length) {
				goto(resourceHref(results[focused]));
				open = false;
			} else if (query.trim()) {
				goto(searchHref(query));
				open = false;
			}
		}
	}
</script>

{#if open}
	<button
		class="fixed inset-0 bg-black/60 z-40"
		onclick={() => (open = false)}
		aria-label="Close search"
	></button>

	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed top-[15%] left-1/2 -translate-x-1/2 w-full max-w-xl z-50
		       bg-zinc-900 border border-zinc-700 rounded-lg shadow-2xl overflow-hidden"
		onkeydown={onKeydown}
	>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			type="text"
			bind:value={query}
			oninput={onInput}
			placeholder="Search the vault..."
			class="w-full px-4 py-3 bg-transparent text-zinc-100 text-sm border-b border-zinc-800 outline-none placeholder:text-zinc-500"
			autofocus
		/>

		{#if failed && !loading}
			<!-- Same register as FilterBar's "contexts unavailable": a read that failed is not
			     a claim about what the vault contains, so it never renders as "No results". -->
			<div class="px-4 py-6 text-sm text-zinc-500 text-center">Search unavailable</div>
		{:else if results.length > 0}
			<div class="max-h-80 overflow-y-auto">
				{#each results as row, i}
					<button
						class="w-full text-left px-4 py-2.5 flex flex-col gap-0.5 transition-colors
						       {i === focused ? 'bg-zinc-800' : 'hover:bg-zinc-800/50'}"
						onclick={() => {
							goto(resourceHref(row));
							open = false;
						}}
					>
						<span class="text-sm text-zinc-100">{row.title}</span>
						<span class="text-xs text-zinc-500"
							>{row.context_name} &middot; {row.doc_type_name}{#if row.managed_meta['temper-stage']}
								&nbsp;&middot; {row.managed_meta['temper-stage']}{/if}</span
						>
					</button>
				{/each}
			</div>
			{#if total > results.length}
				<button
					class="w-full text-left px-4 py-2 text-xs text-quiet-accent hover:bg-zinc-800/50 border-t border-zinc-800"
					onclick={() => {
						goto(searchHref(query));
						open = false;
					}}
				>
					See all {total} results
				</button>
			{/if}
		{:else if query.trim() && !loading}
			<div class="px-4 py-6 text-sm text-zinc-500 text-center">No results</div>
		{/if}
	</div>
{/if}
