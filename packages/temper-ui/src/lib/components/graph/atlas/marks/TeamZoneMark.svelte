<script lang="ts">
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

<g class="team-zone" role="button" tabindex="0" onclick={onEnter} onkeydown={(e) => e.key === 'Enter' && onEnter()} style="cursor:pointer">
	<rect
		{x}
		{y}
		{width}
		{height}
		rx="10"
		fill="#6fa8c7"
		fill-opacity="0.07"
		stroke="#6fa8c7"
		stroke-opacity="0.5"
		stroke-width="1.5"
		stroke-dasharray="6 4"
	/>
	<text x={x + 10} y={y + 18} fill="#9fc4d6" font-size="11" font-weight="600">▸ {name} ⏎</text>
	<text x={x + 10} y={y + 32} fill="#5f7686" font-size="9">{resourceCount} nodes</text>
</g>
