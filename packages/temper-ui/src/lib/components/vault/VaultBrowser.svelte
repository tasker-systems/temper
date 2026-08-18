<!--
	The one vault browser behind all three list routes (`vault/all`,
	`vault/[owner]/[context]`, `vault/search`). Each route now supplies only what
	differs: its heading, whether the context is fixed by the route, and the list it
	read (or the failure it hit).

	Filter state is read from the URL here rather than passed down, so the browser and
	the controls inside it (FilterBar, FacetChips, VaultGrid) all read the same source
	of truth and no route has to thread it through.
-->
<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { page } from '$app/stores';
	import FacetChips from '$lib/components/FacetChips.svelte';
	import RuleHeading from '$lib/components/RuleHeading.svelte';
	import VaultGrid from '$lib/components/VaultGrid.svelte';
	import FilterBar from '$lib/components/vault/FilterBar.svelte';
	import type { ContextRowWithCounts } from '$lib/types';
	import { columnsFor } from '$lib/vault-columns';
	import { activeFilterCount, parseFilters, revealedKind } from '$lib/vault-filters';
	import type { VaultList } from '$lib/vault-list';

	interface Props {
		/** Heading title — the route's own name for what is being browsed. */
		title: string;
		/**
		 * The page that was read, or `null` when the read failed. `null` is not an empty
		 * page: nothing below claims a count, and the grid is not rendered at all.
		 */
		list: VaultList | null;
		/**
		 * Why the read failed, when it did. Rendered in place of the grid. Present exactly
		 * when `list` is `null`.
		 */
		loadError?: string | null;
		/** Contexts for the Context select, from the `(app)` layout's already-loaded copy. */
		contexts: ContextRowWithCounts[];
		/** The route pins `context_ref` itself, so the Context select is not offered. */
		fixedContext?: boolean;
		/** Leading caption segment, e.g. the owner on a context page. */
		captionPrefix?: string | undefined;
	}

	let {
		title,
		list,
		loadError = null,
		contexts,
		fixedContext = false,
		captionPrefix = undefined
	}: Props = $props();

	let filters = $derived(parseFilters($page.url));
	// `revealedKind` reads the doc-type histogram; with no page read there is no histogram,
	// so nothing is revealed and the mixed-kind column set stands.
	let revealed = $derived(revealedKind(filters, list?.facets.doc_type ?? {}));
	let columns = $derived(columnsFor(revealed));

	// No caption on a failed read — every caption here is a count, and a failed read has none.
	let caption = $derived.by(() => {
		if (!list) return undefined;
		const noun = list.total === 1 ? 'resource' : 'resources';
		const body =
			activeFilterCount(filters) > 0
				? `${list.total} matching ${noun}`
				: `${list.total} ${noun}`;
		return captionPrefix ? `${captionPrefix} · ${body}` : body;
	});
</script>

<div class="flex flex-col gap-4 p-6">
	<RuleHeading {title} {caption} />

	<FilterBar {filters} {revealed} {fixedContext} {contexts} />

	{#if list}
		<FacetChips facets={list.facets.doc_type} />

		<VaultGrid
			rows={list.rows}
			{columns}
			total={list.total}
			returned={list.returned}
			truncated={list.truncated}
			limit={list.limit}
			offset={list.offset}
		/>
	{:else}
		<div
			role="alert"
			class="flex flex-col items-start gap-2 rounded border border-amber-500/40 bg-amber-500/5 px-4 py-3"
		>
			<p class="text-sm text-amber-200">
				{loadError ?? 'The vault could not be read.'}
			</p>
			<p class="text-xs text-zinc-400">
				Nothing was retrieved, so nothing here says whether a matching resource exists.
				This is a failed read, not an empty result.
			</p>
			<button
				type="button"
				class="rounded border border-zinc-700 px-3 py-1 text-xs text-quiet-accent hover:text-quiet-fg hover:border-quiet-border"
				onclick={() => invalidateAll()}
			>
				Try again
			</button>
		</div>
	{/if}
</div>
