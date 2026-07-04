<script lang="ts">
	import { TEAM_ZONE } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		width: number;
		height: number;
		name: string;
		resourceCount: number;
		onEnter: () => void;
	}
	let { x, y, width, height, name, resourceCount, onEnter }: Props = $props();
</script>

<g
	class="team-zone"
	role="button"
	tabindex="0"
	onclick={onEnter}
	onkeydown={(e) => {
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			onEnter();
		}
	}}
	style="cursor:pointer"
>
	<rect
		{x}
		{y}
		{width}
		{height}
		rx="10"
		fill={TEAM_ZONE.fill}
		fill-opacity="0.07"
		stroke={TEAM_ZONE.fill}
		stroke-opacity="0.5"
		stroke-width="1.5"
		stroke-dasharray="6 4"
	/>
	<text x={x + 10} y={y + 18} fill={TEAM_ZONE.label} font-size="11" font-weight="600">▸ {name} ⏎</text>
	<text x={x + 10} y={y + 32} fill={TEAM_ZONE.sub} font-size="9">{resourceCount} nodes</text>
</g>
