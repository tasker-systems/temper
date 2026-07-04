<script lang="ts">
	import type { Territory } from '$lib/types/generated/graph_territory';
	import { TERRITORY_TINTS } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		kind: Territory['kind'];
		label: string | null;
		onEnter?: () => void;
	}
	let { x, y, r, kind, label, onEnter }: Props = $props();

	// Region = warm-neutral tint; context = cool tint; cogmap = warm tint. Low-opacity
	// washes with a dashed hull outline, cartographic style.
	const tint = $derived(TERRITORY_TINTS[kind]);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="territory"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	aria-label={label ?? kind}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	style={onEnter ? 'cursor:pointer' : undefined}
>
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
