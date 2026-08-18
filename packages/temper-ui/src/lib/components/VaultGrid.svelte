<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { Grid, WillowDark } from 'wx-svelte-grid';
	import type { ResourceView } from '$lib/types';
	import { orphanSort, type VaultColumn, visibleSortFields } from '$lib/vault-columns';
	import { activeFilterCount, buildFilterUrl, parseFilters } from '$lib/vault-filters';
	import { pageState } from '$lib/vault-list';
	import { resourceHref } from '$lib/vault-url';

	interface Props {
		rows: ResourceView[];
		columns: VaultColumn[];
		total: number;
		returned: number;
		truncated: boolean;
		/**
		 * The page size the server APPLIED (envelope, not request), or `null` for uncapped.
		 * Never re-derived from the URL here — see `toVaultList` for the drift that caused.
		 */
		limit?: number | null;
		/** The offset the server APPLIED — already floored at 0. */
		offset?: number;
	}

	let { rows, columns, total, returned, truncated, limit = null, offset = 0 }: Props = $props();

	// Door-facing sort fields the visible columns can actually carry an indicator for.
	let sortableFields = $derived(visibleSortFields(columns));

	function shortDate(iso: string): string {
		return new Date(iso)
			.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
			.toUpperCase();
	}

	// Derive current sort state from URL to show the active sort indicator. The URL carries the
	// door-facing sort field (e.g. `stage`), but the grid marks sort by column id (`temper-stage`)
	// — map back through the same `sortKey ?? id` correspondence `handleSort` writes with, or the
	// indicator lands on no column and silently disappears.
	let sortMarks = $derived.by(() => {
		const sortField = $page.url.searchParams.get('sort');
		const order = $page.url.searchParams.get('order') as 'asc' | 'desc' | null;
		if (!sortField || !sortableFields.has(sortField)) return {};
		const column = columns.find((c) => (c.sortKey ?? c.id) === sortField);
		if (!column) return {};
		return { [column.id]: { order: order ?? 'desc' } };
	});

	// Transform rows for display
	let gridData = $derived(
		rows.map((r) => {
			// Any column reading a managed key (`temper-*` id) reads it straight off
			// `managed_meta` under that same key — the always-present managed tier on `ResourceView`.
			const managed: Record<string, string> = {};
			for (const column of columns) {
				if (!column.id.startsWith('temper-')) continue;
				const raw = (r.managed_meta as unknown as Record<string, unknown>)[column.id];
				managed[column.id] = raw == null ? '' : String(raw);
			}
			return {
				...r,
				id: r.id,
				updated: shortDate(r.updated),
				_raw_updated: r.updated,
				...managed
			};
		})
	);

	// Map from grid row ID → original ResourceView for navigation
	let rowLookup = $derived(new Map(rows.map((r) => [r.id, r])));

	// Paging chrome, derived from the page the server actually returned. Every number here
	// comes out of the envelope; none of it is recomputed from the request URL.
	let paging = $derived(pageState({ total, returned, truncated, limit, offset }));

	// An active sort no visible column can mark — see `orphanSort`. Rendered as its own chip
	// so the ordering the door is applying is never invisible.
	let orphaned = $derived(
		orphanSort(
			$page.url.searchParams.get('sort'),
			$page.url.searchParams.get('order'),
			sortableFields
		)
	);
	let clearSortHref = $derived.by(() => {
		const url = new URL($page.url);
		url.searchParams.delete('sort');
		url.searchParams.delete('order');
		url.searchParams.delete('offset');
		return `${url.pathname}${url.search}`;
	});

	// Filter state, for the empty-state message
	let filters = $derived(parseFilters($page.url));
	let filterCount = $derived(activeFilterCount(filters));
	let clearFiltersHref = $derived(
		buildFilterUrl($page.url, {
			docTypes: [],
			stage: null,
			status: null,
			contextRef: null,
			q: null,
			tags: []
		})
	);

	function handleFocusCell(ev: {
		row?: string | number;
		column?: string | number;
		eventSource?: string;
	}) {
		if (ev.eventSource !== 'click' || !ev.row) return;
		const row = rowLookup.get(String(ev.row));
		if (!row) return;
		goto(resourceHref(row));
	}

	function handleSort(ev: { key: string | number; order?: 'asc' | 'desc' }) {
		const columnId = String(ev.key);
		const column = columns.find((c) => c.id === columnId);
		// The door-facing sort field, when it differs from the grid column id (e.g. the revealed
		// stage column: id `temper-stage`, door field `stage`). Sending the id itself would be
		// rejected as an invalid `ResourceSortField` enum value.
		const sortField = column?.sortKey ?? columnId;
		if (!sortableFields.has(sortField)) return;
		const url = new URL($page.url);
		url.searchParams.set('sort', sortField);
		url.searchParams.set('order', ev.order === 'asc' ? 'asc' : 'desc');
		url.searchParams.delete('offset');
		goto(url.toString(), { replaceState: true });
	}

	function goToPage(newOffset: number) {
		const url = new URL($page.url);
		if (newOffset > 0) {
			url.searchParams.set('offset', String(newOffset));
		} else {
			url.searchParams.delete('offset');
		}
		goto(url.toString(), { replaceState: true });
	}
</script>

<div class="vault-grid-wrapper">
	{#if rows.length === 0}
		<div class="flex flex-col items-center justify-center gap-3 py-16 text-zinc-500">
			{#if filterCount > 0}
				<p class="text-sm">
					No resources match the current filter{filterCount > 1 ? 's' : ''}.
				</p>
				<a href={clearFiltersHref} class="text-xs underline">Clear filters</a>
			{:else}
				<p class="text-sm">No resources found.</p>
			{/if}
		</div>
	{:else}
		<div class="grid-chrome">
			<div class="flex items-center gap-3">
				<span class="text-xs text-zinc-500 font-mono tracking-wide">
					{paging.rangeStart}–{paging.rangeEnd} of {total}
				</span>
				{#if orphaned}
					<span
						class="inline-flex items-center gap-1.5 rounded border border-zinc-700/50 bg-zinc-800/50 px-2 py-0.5 text-[11px] font-mono text-zinc-400"
					>
						sorted by {orphaned.field} {orphaned.order}
						<a
							href={clearSortHref}
							class="text-zinc-500 hover:text-zinc-200"
							aria-label="Clear sort">×</a
						>
					</span>
				{/if}
			</div>
			{#if paging.paged}
				<div class="pagination">
					<button
						class="page-btn"
						disabled={!paging.hasPrev}
						onclick={() => goToPage(paging.prevOffset)}
						aria-label="Previous page">&larr;</button
					>
					<span class="text-xs text-zinc-500 font-mono tabular-nums"
						>{paging.currentPage}/{paging.totalPages}</span
					>
					<button
						class="page-btn"
						disabled={!paging.hasNext}
						onclick={() => goToPage(paging.nextOffset)}
						aria-label="Next page">&rarr;</button
					>
				</div>
			{/if}
		</div>
		<div class="grid-container">
			<WillowDark fonts={false}>
				<Grid
					data={gridData}
					{columns}
					{sortMarks}
					onfocuscell={handleFocusCell}
					onsortrows={handleSort}
					select={false}
					filterValues={{}}
				/>
			</WillowDark>
		</div>
	{/if}
</div>

<style>
	.vault-grid-wrapper {
		width: 100%;
	}

	.grid-chrome {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 0.5rem;
	}

	.pagination {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.page-btn {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		color: var(--color-quiet-mid);
		background: none;
		border: 1px solid var(--color-quiet-rule);
		border-radius: 3px;
		padding: 0.2rem 0.5rem;
		cursor: pointer;
		transition: color 0.15s, border-color 0.15s;
	}
	.page-btn:hover:not(:disabled) {
		color: var(--color-quiet-fg);
		border-color: var(--color-quiet-border);
	}
	.page-btn:disabled {
		opacity: 0.3;
		cursor: default;
	}

	.grid-container {
		width: 100%;
		height: calc(100vh - 12rem);
	}

	/* ── Theme overrides: blend SVAR WillowDark into Quiet Instrument ── */

	.grid-container :global(.wx-willow-dark-theme) {
		--wx-background: transparent;
		--wx-background-alt: rgba(255, 255, 255, 0.03);
		--wx-background-hover: rgba(255, 255, 255, 0.05);
		--wx-color-font: var(--color-quiet-fg);
		--wx-color-font-alt: var(--color-quiet-dim);
		--wx-color-primary: var(--color-quiet-accent);
		--wx-color-primary-selected: rgba(126, 184, 218, 0.12);
		--wx-border: 1px solid var(--color-quiet-rule);
		--wx-font-family: var(--font-sans);
		--wx-font-size: 13px;
		--wx-line-height: 20px;
		--wx-table-header-background: #0c0c11;
		--wx-table-select-background: rgba(126, 184, 218, 0.08);
		--wx-table-select-border: inset 3px 0 var(--color-quiet-accent);
		--wx-table-border: 1px solid var(--color-quiet-rule);
		--wx-table-header-border: 1px solid rgba(255, 255, 255, 0.08);
		--wx-table-header-cell-border: none;
		--wx-table-cell-border: 1px solid var(--color-quiet-rule);
		--wx-header-font-weight: 500;
		--wx-icon-color: var(--color-quiet-dim);
	}

	/* Row hover — pointer cursor and subtle highlight */
	.grid-container :global(.wx-row) {
		cursor: pointer;
		transition: background-color 0.12s ease;
	}
	.grid-container :global(.wx-row:hover) {
		background-color: rgba(255, 255, 255, 0.04);
	}

	/* Header sort indicator — slightly brighter when active */
	.grid-container :global(.wx-sort i) {
		color: var(--color-quiet-accent);
		opacity: 0.85;
	}

	/* Sortable headers get a subtle hover cue */
	.grid-container :global(.wx-h-row .wx-cell:has(.wx-sort)) {
		cursor: pointer;
	}
	.grid-container :global(.wx-h-row .wx-cell:has(.wx-sort):hover) {
		color: var(--color-quiet-fg);
	}
</style>
