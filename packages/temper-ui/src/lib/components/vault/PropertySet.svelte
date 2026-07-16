<script lang="ts">
	import type { PropertyRow } from '$lib/properties';
	import PropertyValue from './PropertyValue.svelte';

	let { rows }: { rows: PropertyRow[] } = $props();

	// The rule between the managed run and the open run. Managed keys always
	// lead (mergeProperties guarantees the order), so this is the first open row.
	let firstOpenKey = $derived(rows.find((r) => !r.managed)?.key ?? null);
</script>

<div class="props">
	<div class="label">Properties · {rows.length}</div>
	<dl>
		{#each rows as row (row.key)}
			{#if row.key === firstOpenKey}
				<hr />
			{/if}
			<div class="row" class:is-managed={row.managed}>
				<dt>{row.key}</dt>
				<dd><PropertyValue value={row.value} /></dd>
			</div>
		{/each}
	</dl>
</div>

<style>
	.props {
		padding: 14px 22px 16px;
		border-bottom: 1px solid var(--color-quiet-rule);
		background: rgba(255, 255, 255, 0.015);
	}
	.label {
		font-family: var(--font-mono);
		font-size: 9px;
		letter-spacing: var(--track-label);
		text-transform: uppercase;
		color: var(--color-quiet-dim);
		margin-bottom: 9px;
	}
	dl {
		margin: 0;
	}
	.row {
		display: grid;
		grid-template-columns: 132px 1fr;
		gap: 10px;
		padding: 3px 0;
		align-items: start;
	}
	dt {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--color-quiet-dim);
	}
	dd {
		margin: 0;
		min-width: 0;
	}
	/* Managed keys tint toward the doc-type hue; open keys stay neutral (spec D2). */
	.row.is-managed dt {
		color: color-mix(in srgb, var(--hue) 52%, var(--color-quiet-dim));
	}
	hr {
		border: 0;
		border-top: 1px dashed rgba(255, 255, 255, 0.1);
		margin: 8px 0;
	}
</style>
