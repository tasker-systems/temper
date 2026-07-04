<script lang="ts">
	import type { AtlasEdge } from '$lib/types/generated/graph_atlas';
	import { edgeStyle } from '$lib/graph/atlas/palette';

	interface Props {
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		edge: AtlasEdge;
		label?: boolean;
	}
	let { x1, y1, x2, y2, edge, label = false }: Props = $props();

	const s = $derived(edgeStyle(edge));
	const midX = $derived((x1 + x2) / 2);
	const midY = $derived((y1 + y2) / 2);
</script>

<g class="edge">
	<line
		{x1}
		{y1}
		{x2}
		{y2}
		stroke={s.color}
		stroke-width={s.width}
		stroke-dasharray={s.dash ?? undefined}
		marker-end={s.markerEnd ? 'url(#arrow-end)' : undefined}
		marker-start={s.markerStart ? 'url(#arrow-start)' : undefined}
	/>
	{#if label && edge.label}
		<text x={midX} y={midY - 3} text-anchor="middle" fill="#c9b183" font-size="9">{edge.label}</text>
	{/if}
</g>
