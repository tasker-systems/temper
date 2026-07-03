<script lang="ts">
	import type { Territory } from '$lib/types/generated/graph_territory';

	interface Props {
		x: number;
		y: number;
		r: number;
		kind: Territory['kind'];
		label: string | null;
	}
	let { x, y, r, kind, label }: Props = $props();

	// Region = warm-neutral tint; context = cool tint; cogmap = warm tint. Low-opacity
	// washes with a dashed hull outline, cartographic style.
	const TINTS: Record<Territory['kind'], string> = {
		region: '#e0b060',
		context: '#6fa8c7',
		cogmap: '#e8942e'
	};
	const tint = $derived(TINTS[kind]);
</script>

<g class="territory">
	<circle
		cx={x}
		cy={y}
		{r}
		fill={tint}
		fill-opacity="0.09"
		stroke={tint}
		stroke-opacity="0.4"
		stroke-width="1.5"
		stroke-dasharray="6 4"
	/>
	{#if label}
		<text
			x={x}
			y={y}
			text-anchor="middle"
			fill={tint}
			font-size="11"
			font-weight="600"
			letter-spacing="1"
			style="text-transform:uppercase"
		>
			{label}
		</text>
	{/if}
</g>
