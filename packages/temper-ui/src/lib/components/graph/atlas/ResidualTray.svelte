<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/stores';
	import type { ResidualBucket } from '$lib/types/generated/graph_context';
	import { buildDrillBucketUrl } from '$lib/graph/atlas/nav';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';
	import { trayModel } from '$lib/graph/atlas/residualTray';

	interface Props {
		/** The residue that reaches no container, one cell per group-key value. */
		buckets: ResidualBucket[];
		/** The group key these buckets partition (`doc_type` by default). */
		groupKey: string;
		/** Available layout width; cells share it by count, clamped to a legible floor. */
		width: number;
	}
	let { buckets, groupKey, width }: Props = $props();

	// The tray is a doorway, NOT a landmark: it lives in page chrome, outside the force
	// field, so residuals never drag the survey's salience toward zero (spec D2). On
	// well-edged data `trayModel` returns [] and the tray simply does not render.
	const cells = $derived(trayModel(buckets, width));

	// Cool tint: a residual bucket is still the builder axis, and temperature encodes the
	// axis (spec D6) — the context tint, never the warm region/cogmap hues.
	const tint = TERRITORY_TINTS.context;

	function enter(value: string) {
		goto(buildDrillBucketUrl($page.url, groupKey, value));
	}
</script>

{#if cells.length > 0}
	<nav class="residual-tray" aria-label="Unfiled resources" style={`--tray-tint: ${tint};`}>
		{#each cells as cell (cell.value)}
			<button
				type="button"
				class="tray-cell"
				style={`width: ${cell.width}px;`}
				onclick={() => enter(cell.value)}
			>
				<span class="cell-value" title={cell.value}>{cell.value}</span>
				<span class="cell-count">{cell.count} resources</span>
			</button>
		{/each}
	</nav>
{/if}

<style>
	/* A horizontal rail of doorways. When many tiny buckets each clamp to MIN_CELL the
	   total exceeds the tray width, so the rail scrolls — never the page (no page-level
	   horizontal scroll). */
	.residual-tray {
		display: flex;
		flex-direction: row;
		align-items: stretch;
		gap: 6px;
		width: 100%;
		max-width: 100%;
		overflow-x: auto;
		overflow-y: hidden;
		padding: 4px 2px 6px;
		box-sizing: border-box;
	}

	.tray-cell {
		flex: 0 0 auto;
		display: flex;
		flex-direction: column;
		gap: 3px;
		min-width: 0;
		padding: 8px 12px;
		text-align: left;
		cursor: pointer;
		border: 1px solid color-mix(in srgb, var(--tray-tint) 45%, transparent);
		border-radius: 8px;
		background: color-mix(in srgb, var(--tray-tint) 12%, transparent);
		color: #e6ebf2;
		font: inherit;
		box-sizing: border-box;
	}

	.cell-value {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-size: 13px;
		font-weight: 600;
	}

	.cell-count {
		font-size: 11px;
		color: #9aa5b5;
		/* Counts align digit-for-digit so buckets scan as a column of numbers. */
		font-variant-numeric: tabular-nums;
	}

	.tray-cell:hover {
		background: color-mix(in srgb, var(--tray-tint) 20%, transparent);
		border-color: color-mix(in srgb, var(--tray-tint) 65%, transparent);
	}

	.tray-cell:focus-visible {
		outline: 2px solid #8ecbff;
		outline-offset: 2px;
	}

	/* Only animate when the viewer allows motion. */
	@media (prefers-reduced-motion: no-preference) {
		.tray-cell {
			transition:
				background-color 120ms ease,
				border-color 120ms ease;
		}
	}
</style>
