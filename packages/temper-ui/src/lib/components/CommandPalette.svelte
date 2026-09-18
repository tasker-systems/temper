<script lang="ts">
	import { goto } from '$app/navigation';
	import type { ResourceView } from '$lib/types';
	import type { SearchResponse } from '$lib/types/generated/search';
	import { SEARCH_PAGE_SIZE } from '$lib/search-page-size';
	import { resourceHref } from '$lib/vault-url';

	let open = $state(false);
	let query = $state('');
	let answer = $state<SearchResponse | null>(null);
	// Which arm of the answer renders. The answer always carries both; the door is never asked
	// to compute only one, and the switch below never re-runs the read — the selected arm is
	// already in the response the current query returned.
	let exactMode = $state(false);
	let focused = $state(0);
	let loading = $state(false);
	let failed = $state(false);
	let debounceTimer: ReturnType<typeof setTimeout>;

	// D7 — the palette poses single questions. A reader who typed several gets a decline that
	// says so, never a narrower answer to whichever part the door happened to match. The rule
	// is bounded on purpose: punctuation that SEPARATES questions ("?" or ";" with something
	// after it) declines; one trailing terminator, double punctuation, and "and" do not —
	// "search and rescue" is one question.
	function namesSeveralQuestions(q: string): boolean {
		const body = q.trim().replace(/[?;\s]+$/, '');
		return body.includes('?') || body.includes(';');
	}

	const arm = $derived(answer && (exactMode ? answer.exact : answer.wide));
	const hits = $derived(arm?.hits ?? []);
	const declined = $derived(namesSeveralQuestions(query));
	// Degraded belongs to the wide arm alone — the exact arm has no such case — so the face is
	// decided here rather than by touching a field the union only sometimes carries.
	const wideDegraded = $derived(!exactMode && (answer?.wide.degraded ?? false));

	export function toggle() {
		open = !open;
		if (open) {
			query = '';
			answer = null;
			exactMode = false;
			focused = 0;
			failed = false;
		}
	}

	async function search(q: string) {
		if (!q.trim() || namesSeveralQuestions(q)) {
			answer = null;
			failed = false;
			focused = 0;
			return;
		}
		loading = true;
		try {
			const resp = await fetch('/_internal/search', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ q }),
			});
			if (!resp.ok) {
				failed = true;
				answer = null;
			} else {
				failed = false;
				answer = await resp.json();
			}
		} catch {
			failed = true;
			answer = null;
		}
		focused = 0;
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
			focused = Math.min(focused + 1, hits.length);
		} else if (e.key === 'ArrowUp') {
			e.preventDefault();
			focused = Math.max(focused - 1, 0);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			if (focused < hits.length) {
				goto(resourceHref(hits[focused].resource as ResourceView));
				open = false;
			}
			// No bare-Enter hand-off (D6): the list door locates, it does not answer, and
			// routing this question there would be the substitution defect restated as a route.
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

		<!-- The arm the answer is being read from, named in reader terms (D4), with the Exact
		     switch beside it. The header still promises "Search the vault…" — this is what makes
		     the promise true. -->
		<div class="flex items-center justify-between px-4 py-1.5 border-b border-zinc-800">
			<span class="text-xs text-zinc-500" data-testid="arm-caption"
				>{exactMode ? 'matching these words' : 'similar in meaning'}</span
			>
			<label class="flex items-center gap-1.5 text-xs text-zinc-300 cursor-pointer select-none">
				<span>Exact</span>
				<input type="checkbox" bind:checked={exactMode} class="accent-zinc-500" />
			</label>
		</div>

		{#if failed && !loading}
			<!-- Same register as FilterBar's "contexts unavailable": a read that failed is not
			     a claim about what the vault contains, so it never renders as "No results". -->
			<div class="px-4 py-6 text-sm text-zinc-500 text-center">Search unavailable</div>
		{:else if declined && !loading}
			<!-- D7 — asked for several at once, the palette declines saying so instead of
			     answering whichever question matched first. -->
			<div class="px-4 py-6 text-sm text-zinc-500 text-center">
				One question at a time — this search answers a single question. Ask them one by one.
			</div>
		{:else if arm}
			{#if wideDegraded}
				<!-- FIRST, ahead of any empty face: a degraded arm arrives with zero hits and a
				     reason that would otherwise read as "nothing matched" — but nothing about the
				     corpus was learned. The wide arm says so here rather than returning an empty
				     list that reads like an answer, and the Exact switch is offered as the
				     reader's EXPLICIT action. The system never flips it on its own. -->
				<div class="px-4 py-6 text-sm text-zinc-500 text-center">
					<p>Meaning-match couldn't run just now</p>
					<button
						class="mt-2 text-xs text-quiet-accent hover:text-zinc-200"
						onclick={() => (exactMode = true)}
					>
						Exact matches words directly — use it
					</button>
				</div>
			{:else if arm.reason === 'no_match'}
				<!-- Rephrasing can help here: the corpus was reachable, the words were not. -->
				<div class="px-4 py-6 text-sm text-zinc-500 text-center">
					Nothing matched — try different words
				</div>
			{:else if arm.reason === 'out_of_scope'}
				<!-- Scope-shaped emptiness: rephrasing will never help, and the face says why
				     instead of rendering a plain zero. -->
				<div class="px-4 py-6 text-sm text-zinc-500 text-center">
					No phrasing will reach this — the answer is not inside what your vault shows you
				</div>
			{:else if hits.length > 0}
				<div class="max-h-80 overflow-y-auto">
					{#each hits as hit, i}
						<button
							class="w-full text-left px-4 py-2.5 flex flex-col gap-0.5 transition-colors
							       {i === focused ? 'bg-zinc-800' : 'hover:bg-zinc-800/50'}"
							onclick={() => {
								goto(resourceHref(hit.resource as ResourceView));
								open = false;
							}}
						>
							<span class="text-sm text-zinc-100">{hit.resource.title}</span>
							<span class="text-xs text-zinc-500"
								>{hit.resource.context_name} &middot;
								{hit.resource.doc_type_name}{#if hit.resource.managed_meta['temper-stage']}
									&nbsp;&middot; {hit.resource.managed_meta['temper-stage']}{/if}</span
							>
						</button>
					{/each}
				</div>
				{#if hits.length >= SEARCH_PAGE_SIZE}
					<!-- D6 — the palette states its bound instead of pretending the answer is whole. -->
					<div
						class="px-4 py-2 text-xs text-zinc-500 border-t border-zinc-800"
					>
						Showing the first {SEARCH_PAGE_SIZE}
					</div>
				{/if}
			{:else}
				<div class="px-4 py-6 text-sm text-zinc-500 text-center">No results</div>
			{/if}
		{/if}
	</div>
{/if}
