<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { Grid, WillowDark } from 'wx-svelte-grid';
	import type { ResourceRow, ResourceSortField } from '$lib/types';
	import { resourceHref } from '$lib/vault-url';

	interface Props {
		rows: ResourceRow[];
		total: number;
		limit?: number;
		offset?: number;
	}

	let { rows, total, limit = 50, offset = 0 }: Props = $props();

	// Sortable fields that the backend supports
	const SORTABLE: Set<string> = new Set<ResourceSortField>([
		'updated',
		'created',
		'title',
		'stage',
		'seq',
		'context_name',
		'doc_type_name'
	]);

	function shortDate(iso: string): string {
		return new Date(iso)
			.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
			.toUpperCase();
	}

	const columns = [
		{ id: 'title', header: 'Title', flexgrow: 1, sort: true },
		{ id: 'context_name', header: 'Context', width: 140, sort: true },
		{ id: 'doc_type_name', header: 'Type', width: 120, sort: true },
		{ id: 'stage', header: 'Stage', width: 100, sort: true },
		{ id: 'updated', header: 'Updated', width: 110, sort: true }
	];

	// Derive current sort state from URL to show the active sort indicator
	let sortMarks = $derived.by(() => {
		const key = $page.url.searchParams.get('sort');
		const order = $page.url.searchParams.get('order') as 'asc' | 'desc' | null;
		if (key && SORTABLE.has(key)) {
			return { [key]: { order: order ?? 'desc' } };
		}
		return {};
	});

	// Transform rows for display
	let gridData = $derived(
		rows.map((r) => ({
			...r,
			id: r.id,
			updated: shortDate(r.updated),
			_raw_updated: r.updated,
			stage: r.stage ?? '',
			seq: r.seq ?? ''
		}))
	);

	// Map from grid row ID → original ResourceRow for navigation
	let rowLookup = $derived(new Map(rows.map((r) => [r.id, r])));

	// Pagination
	let currentPage = $derived(Math.floor(offset / limit) + 1);
	let totalPages = $derived(Math.ceil(total / limit));
	let hasPrev = $derived(offset > 0);
	let hasNext = $derived(offset + limit < total);

	function handleFocusCell(ev: {
		row?: string | number;
		column?: string | number;
		eventSource?: string;
	}) {
		if (ev.eventSource !== 'click' || !ev.row) return;
		const row = rowLookup.get(String(ev.row));
		if (!row) return;
		const href = resourceHref(row);
		if (href) goto(href);
	}

	function handleSort(ev: { key: string | number; order?: 'asc' | 'desc' }) {
		const key = String(ev.key);
		if (!SORTABLE.has(key)) return;
		const url = new URL($page.url);
		url.searchParams.set('sort', key);
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
			<p class="text-sm">No resources found.</p>
		</div>
	{:else}
		<div class="grid-chrome">
			<div class="text-xs text-zinc-500 font-mono tracking-wide">
				{offset + 1}–{Math.min(offset + rows.length, total)} of {total}
			</div>
			{#if totalPages > 1}
				<div class="pagination">
					<button
						class="page-btn"
						disabled={!hasPrev}
						onclick={() => goToPage(offset - limit)}
						aria-label="Previous page">&larr;</button
					>
					<span class="text-xs text-zinc-500 font-mono tabular-nums"
						>{currentPage}/{totalPages}</span
					>
					<button
						class="page-btn"
						disabled={!hasNext}
						onclick={() => goToPage(offset + limit)}
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
