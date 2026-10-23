<script lang="ts">
	import { type EdgeMark, edgeStyle } from '$lib/graph/palette';

	interface Props {
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		/**
		 * Structural, not `AtlasEdge` `[widened — 2026-08-20]`. The successor surface's edges are
		 * `ViaEntry` rows and carry no stored weight; both shapes satisfy `EdgeMark`, so one mark
		 * serves both without an adapter inventing the field the walk does not report.
		 */
		edge: EdgeMark;
		label?: boolean;
		onSelect?: () => void;
	}
	let { x1, y1, x2, y2, edge, label = false, onSelect }: Props = $props();

	const s = $derived(edgeStyle(edge));
	const midX = $derived((x1 + x2) / 2);
	const midY = $derived((y1 + y2) / 2);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="edge atlas-focusable"
	role={onSelect ? 'button' : undefined}
	tabindex={onSelect ? 0 : undefined}
	aria-label={edge.label ?? edge.edge_kind}
	onclick={onSelect}
	onkeydown={(e) => e.key === 'Enter' && onSelect?.()}
	style={onSelect ? 'cursor:pointer' : undefined}
>
	<line {x1} {y1} {x2} {y2} stroke="transparent" stroke-width="10" />
	<!-- Focus halo behind the visible stroke so the edge colour stays on top. -->
	<line class="focus-ring" {x1} {y1} {x2} {y2} stroke-width={s.width + 5} stroke-linecap="round" />
	<line
		{x1}
		{y1}
		{x2}
		{y2}
		style:stroke={s.color}
		stroke-width={s.width}
		stroke-dasharray={s.dash ?? undefined}
		marker-end={s.markerEnd ? 'url(#arrow-end)' : undefined}
		marker-start={s.markerStart ? 'url(#arrow-start)' : undefined}
	/>
	{#if s.cross}
		<!-- contradicts' channel beyond hue (palette.ts `EdgeStyle.cross`) -->
		<g class="contradicts-cross" transform="translate({midX} {midY})" style:stroke={s.color} stroke-width="1.6" stroke-linecap="round">
			<line x1="-3.5" y1="-3.5" x2="3.5" y2="3.5" />
			<line x1="-3.5" y1="3.5" x2="3.5" y2="-3.5" />
		</g>
	{/if}
	{#if label && edge.label}
		<text x={midX} y={midY - 3} text-anchor="middle" style="fill: var(--tp-accent)" font-size="9">{edge.label}</text>
	{/if}
</g>
