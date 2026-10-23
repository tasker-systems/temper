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
		<div class="flex flex-col items-center justify-center gap-3 py-16 text-tp-text-subtle">
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
				<span class="text-xs text-tp-text-subtle font-mono tracking-wide">
					{paging.rangeStart}–{paging.rangeEnd} of {total}
				</span>
				{#if orphaned}
					<span
						class="inline-flex items-center gap-1.5 rounded border border-tp-rule bg-tp-surface-raised px-2 py-0.5 text-[11px] font-mono text-tp-text-muted"
					>
						sorted by {orphaned.field} {orphaned.order}
						<a
							href={clearSortHref}
							class="text-tp-text-subtle hover:text-tp-text"
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
					<span class="text-xs text-tp-text-subtle font-mono tabular-nums"
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
		color: var(--tp-text-muted);
		background: none;
		border: 1px solid var(--tp-rule-strong);
		border-radius: 3px;
		padding: 0.2rem 0.5rem;
		cursor: pointer;
		transition: color 0.15s, border-color 0.15s;
	}
	.page-btn:hover:not(:disabled) {
		color: var(--tp-text);
		border-color: var(--tp-accent-line);
	}
	.page-btn:disabled {
		opacity: 0.3;
		cursor: default;
	}

	.grid-container {
		width: 100%;
		height: calc(100vh - 12rem);
	}

	/*
	 * The SVAR bridge: WillowDark's variable names, answered from contract roles, so any theme
	 * restyles the grid with no per-theme work (as contract/shadcn-bridge.css does for shadcn).
	 * Every colour-valued variable the two WillowDark files declare with a value of their own is
	 * answered here — the ones they derive (`var(--wx-color-primary)` and the like) follow — and
	 * the two component rules wx-svelte-grid@2.7.0 paints with a literal are answered below, so
	 * no vendor colour is reachable in any grid state, used today or not. Shadows have no role
	 * and are off. Web-local: the desktop does not use SVAR.
	 *
	 * Precedence: the vendor rules are unlayered `.wx-willow-dark-theme` (one class); these are
	 * unlayered and scoped under `.grid-container`, so they win. Moved into `layer(base)`, they
	 * would lose without a sound.
	 *
	 * Three mappings are stand-ins: no role says what they mean, so each takes the nearest role
	 * whose floor still holds, and no grid state renders any of them today. They are marked
	 * "stand-in" where they sit.
	 */
	.grid-container :global(.wx-willow-dark-theme) {
		/* WillowDark fixes `color-scheme: dark`; the grid's scrollbars and native controls follow
		   the theme in force instead. */
		color-scheme: inherit;
		--wx-color-primary: var(--tp-accent);
		--wx-color-primary-selected: var(--tp-accent-wash);
		--wx-color-primary-font: var(--tp-on-accent);
		--wx-color-secondary-hover: var(--tp-accent-hover);
		--wx-color-secondary-border-disabled: var(--tp-rule);
		--wx-color-success: var(--tp-success);
		--wx-color-warning: var(--tp-notice);
		--wx-color-danger: var(--tp-danger);
		/* stand-in: no disabled-fill role; a raised fill reads as unavailable on ground */
		--wx-color-disabled: var(--tp-surface-raised);
		--wx-color-disabled-alt: var(--tp-surface-raised);
		--wx-color-font: var(--tp-text);
		--wx-color-font-alt: var(--tp-text-subtle);
		--wx-color-font-disabled: var(--tp-text-faint);
		--wx-background: transparent;
		--wx-background-alt: var(--tp-surface);
		--wx-background-hover: var(--tp-accent-hover);
		--wx-icon-color: var(--tp-text-subtle);
		--wx-border-color: var(--tp-rule);
		--wx-border: 1px solid var(--tp-rule);
		--wx-border-light: 1px solid var(--tp-rule);
		--wx-border-medium: 1px solid var(--tp-rule);
		--wx-shadow-light: none;
		--wx-box-shadow: none;
		--wx-box-shadow-strong: none;
		--wx-font-family: var(--tp-font-ui);
		--wx-font-size: 13px;
		--wx-line-height: 20px;
		--wx-input-background: var(--tp-surface);
		--wx-input-border: 1px solid var(--tp-rule-strong);
		--wx-input-border-focus: 1px solid var(--tp-accent-line);
		/* stand-in: no text-on-danger role; on-accent's floor is measured on accent, not danger */
		--wx-button-danger-font-color: var(--tp-on-accent);
		--wx-button-pressed: var(--tp-accent-hover);
		--wx-button-danger-pressed: var(--tp-danger);
		--wx-button-primary-pressed: var(--tp-accent);
		--wx-button-box-shadow: none;
		--wx-button-primary-box-shadow: none;
		--wx-tabs-hover-border: var(--tp-rule-strong);
		--wx-switch-background: var(--tp-surface-raised);
		--wx-switch-thumb-border-disabled: 1px solid var(--tp-rule);
		/* stand-in: no control-thumb role; text holds 7:1 on the raised track */
		--wx-switch-thumb-background: var(--tp-text);
		--wx-popup-background: var(--tp-surface-raised);
		--wx-popup-border: 1px solid var(--tp-rule-strong);
		--wx-modal-background: var(--tp-surface-raised);
		--wx-modal-border: 1px solid var(--tp-rule-strong);
		--wx-modal-backdrop: var(--tp-overlay);
		--wx-modal-header-font-color: var(--tp-text);
		--wx-notice-type-icon-color: var(--tp-text-muted);
		--wx-tooltip-font-color: var(--tp-text);
		--wx-tooltip-background: var(--tp-surface-raised);

		/* wx-svelte-grid's own WillowDark */
		--wx-table-header-background: var(--tp-surface);
		--wx-table-select-background: var(--tp-accent-wash);
		--wx-table-select-border: inset 3px 0 var(--tp-accent);
		--wx-table-border: 1px solid var(--tp-rule);
		--wx-table-header-border: 1px solid var(--tp-rule-strong);
		--wx-table-header-cell-border: none;
		--wx-table-cell-border: 1px solid var(--tp-rule);
		--wx-table-editor-dropdown-shadow: none;
		--wx-header-font-weight: 500;
	}
	/* The vendor colours not spelled through a variable: the table menu's shadow (WillowDark),
	   the multi-sort index badge (HeaderCell, reachable by ctrl/cmd-clicking a header) and the
	   search-match fill (Cell, unused here). */
	.grid-container :global(.wx-willow-dark-theme .wx-table-menu) {
		box-shadow: none;
	}
	.grid-container :global(.wx-order) {
		color: var(--tp-on-accent);
		background-color: var(--tp-accent);
	}
	.grid-container :global(.wx-search) {
		background-color: var(--tp-notice-wash);
	}

	/* Row hover — pointer cursor and the hover fill */
	.grid-container :global(.wx-row) {
		cursor: pointer;
		transition: background-color 0.12s ease;
	}
	.grid-container :global(.wx-row:hover) {
		background-color: var(--tp-accent-hover);
	}

	/* The active sort mark */
	.grid-container :global(.wx-sort i) {
		color: var(--tp-accent);
	}

	/* Sortable headers get a hover cue */
	.grid-container :global(.wx-h-row .wx-cell:has(.wx-sort)) {
		cursor: pointer;
	}
	.grid-container :global(.wx-h-row .wx-cell:has(.wx-sort):hover) {
		color: var(--tp-text);
	}
</style>
