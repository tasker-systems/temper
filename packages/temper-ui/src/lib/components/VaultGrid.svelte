<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import { Grid } from 'wx-svelte-grid';
	import type { ResourceRow } from '$lib/types';

	interface Props {
		rows: ResourceRow[];
		total: number;
	}

	let { rows, total }: Props = $props();

	function shortDate(iso: string): string {
		return new Date(iso)
			.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
			.toUpperCase();
	}

	const columns = [
		{ id: 'title', header: 'Title', flexgrow: 1, sort: true },
		{ id: 'context_name', header: 'Context', width: 140, sort: false },
		{ id: 'doc_type_name', header: 'Type', width: 120, sort: false },
		{ id: 'stage', header: 'Stage', width: 100, sort: true },
		{ id: 'updated', header: 'Updated', width: 110, sort: true },
		{ id: 'seq', header: 'Seq', width: 60, sort: true }
	];

	// Transform rows to include formatted dates and ensure id field for the grid
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

	function handleCellClick(ev: { row: Record<string, unknown> }) {
		const row = ev.row as ResourceRow;
		const ident = row.slug || row.id;
		goto(`/vault/${row.owner_handle}/${row.context_name}/${row.doc_type_name}/${ident}`);
	}

	function handleSort(ev: { key: string; order?: string }) {
		const url = new URL($page.url);
		url.searchParams.set('sort', ev.key);
		url.searchParams.set('order', ev.order === 'asc' ? 'asc' : 'desc');
		url.searchParams.delete('offset');
		goto(url.toString(), { replaceState: true });
	}
</script>

<div class="vault-grid-wrapper">
	{#if rows.length === 0}
		<div class="flex flex-col items-center justify-center gap-3 py-16 text-zinc-500">
			<p class="text-sm">No resources found.</p>
		</div>
	{:else}
		{#if total > rows.length}
			<div class="text-xs text-zinc-500 font-mono tracking-wide mb-2">
				Showing {rows.length} of {total}
			</div>
		{/if}
		<div class="grid-container">
			<Grid
				data={gridData}
				{columns}
				oncellclick={handleCellClick}
				onsort={handleSort}
				select={false}
				filterValues={{}}
			/>
		</div>
	{/if}
</div>

<style>
	.vault-grid-wrapper {
		width: 100%;
	}
	.grid-container {
		width: 100%;
		height: calc(100vh - 10rem);
	}
</style>
