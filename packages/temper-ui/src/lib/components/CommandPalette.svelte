<script lang="ts">
	import { goto } from '$app/navigation';
	import type { ResourceView } from '$lib/types';
	import type { SearchResponse } from '$lib/types/generated/search';
	import { uncountedSentence } from '@tasker-systems/temper-design/omission';
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
	// D6 — the palette states its bound on every answer. The search read counts nothing but each
	// arm says whether more matched (`truncated`, from the row it read past the page), never
	// "more" inferred from a full page. The exact arm's `false` means nothing more matched. The
	// wide arm's does not: unscoped, it ranks a bounded draw of nearest chunks before visibility,
	// so `false` says nothing about what lies past the draw, and the sentence says "not reported"
	// rather than "all". An answer from a server that does not send the field is not reported
	// either.
	const bound = $derived.by(() => {
		const more = arm?.truncated;
		if (more === undefined) return null;
		if (more) return { more: true };
		return exactMode ? { more: false } : null;
	});
	const omission = $derived(
		uncountedSentence(
			hits.length,
			`${hits.length === 1 ? 'result' : 'results'} ${exactMode ? 'matching these words' : 'similar in meaning'}`,
			bound
		)
	);
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
		class="fixed inset-0 bg-tp-overlay z-40"
		onclick={() => (open = false)}
		aria-label="Close search"
	></button>

	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="fixed top-[15%] left-1/2 -translate-x-1/2 w-full max-w-xl z-50
		       bg-tp-surface-raised border border-tp-rule-strong rounded-lg shadow-2xl overflow-hidden"
		onkeydown={onKeydown}
	>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			type="text"
			bind:value={query}
			oninput={onInput}
			placeholder="Search the vault..."
			class="w-full px-4 py-3 bg-transparent text-tp-text text-sm border-b border-tp-rule outline-none placeholder:text-tp-text-subtle"
			autofocus
		/>

		<!-- The arm the answer is being read from, named in reader terms (D4), with the Exact
		     switch beside it. The header still promises "Search the vault…" — this is what makes
		     the promise true. -->
		<div class="flex items-center justify-between px-4 py-1.5 border-b border-tp-rule">
			<span class="text-xs text-tp-text-subtle" data-testid="arm-caption"
				>{exactMode ? 'matching these words' : 'similar in meaning'}</span
			>
			<label class="flex items-center gap-1.5 text-xs text-tp-text-muted cursor-pointer select-none">
				<span>Exact</span>
				<input type="checkbox" bind:checked={exactMode} class="accent-tp-accent" />
			</label>
		</div>

		{#if failed && !loading}
			<!-- Same register as FilterBar's "contexts unavailable": a read that failed is not
			     a claim about what the vault contains, so it never renders as "No results". -->
			<div class="px-4 py-6 text-sm text-tp-text-subtle text-center">Search unavailable</div>
		{:else if declined && !loading}
			<!-- D7 — asked for several at once, the palette declines saying so instead of
			     answering whichever question matched first. -->
			<div class="px-4 py-6 text-sm text-tp-text-subtle text-center">
				One question at a time — this search answers a single question. Ask them one by one.
			</div>
		{:else if arm}
			{#if wideDegraded}
				<!-- FIRST, ahead of any empty face: a degraded arm arrives with zero hits and a
				     reason that would otherwise read as "nothing matched" — but nothing about the
				     corpus was learned. The wide arm says so here rather than returning an empty
				     list that reads like an answer, and the Exact switch is offered as the
				     reader's EXPLICIT action. The system never flips it on its own. -->
				<div class="px-4 py-6 text-sm text-tp-text-subtle text-center">
					<p>Meaning-match couldn't run just now</p>
					<button
						class="mt-2 text-xs text-tp-accent hover:text-tp-text"
						onclick={() => (exactMode = true)}
					>
						Exact matches words directly — use it
					</button>
				</div>
			{:else if hits.length === 0 && arm.truncated}
				<!-- Every row of this page fell away after the read (e.g. deleted meanwhile), but the
				     arm says more follow: "nothing matched" would be false of it. Ahead of the
				     `no_match` branch, whose reason is computed after those rows fell away. -->
				<div class="px-4 py-6 text-sm text-tp-text-subtle text-center" data-testid="omission">
					{omission}
				</div>
			{:else if arm.reason === 'no_match'}
				<!-- Rephrasing can help here: the corpus was reachable, the words were not. -->
				<div class="px-4 py-6 text-sm text-tp-text-subtle text-center">
					Nothing matched — try different words
				</div>
			{:else if arm.reason === 'out_of_scope'}
				<!-- Scope-shaped emptiness: rephrasing will never help, and the face says why
				     instead of rendering a plain zero. -->
				<div class="px-4 py-6 text-sm text-tp-text-subtle text-center">
					No phrasing will reach this — the answer is not inside what your vault shows you
				</div>
			{:else if hits.length > 0}
				<div class="max-h-80 overflow-y-auto">
					{#each hits as hit, i}
						{@const home = hit.resource.cogmap_name ?? hit.resource.context_name}
						<button
							class="w-full text-left px-4 py-2.5 flex flex-col gap-0.5 transition-colors
							       {i === focused ? 'bg-tp-accent-wash' : 'hover:bg-tp-accent-hover'}"
							onclick={() => {
								goto(resourceHref(hit.resource as ResourceView));
								open = false;
							}}
						>
							<span class="text-sm text-tp-text">{hit.resource.title}</span>
							<!-- A row is homed by exactly one anchor, so the name of the home it does
							     NOT have is absent from the wire (`skip_serializing_if` — the rule
							     `isCogmapHomed` owns). Read the home name the row actually carries;
							     when it carries neither, render no home segment at all — never a
							     dangling "· doc_type". The prefix is a JS string because the template
							     trims a trailing text space at a block boundary, which is what made
							     the separator collapse onto the doc type. -->
							<span class="text-xs text-tp-text-subtle"
								>{home ? `${home} · ` : ''}{hit.resource.doc_type_name}{#if hit.resource.managed_meta['temper-stage']}
									&nbsp;&middot; {hit.resource.managed_meta['temper-stage']}{/if}</span
							>
						</button>
					{/each}
				</div>
				<div
					class="px-4 py-2 text-xs text-tp-text-subtle border-t border-tp-rule"
					data-testid="omission"
				>
					{omission}
				</div>
			{:else}
				<div class="px-4 py-6 text-sm text-tp-text-subtle text-center">No results</div>
			{/if}
		{/if}
	</div>
{/if}
