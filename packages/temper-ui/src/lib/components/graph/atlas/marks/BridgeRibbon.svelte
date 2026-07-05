<script lang="ts">
	interface Props {
		x1: number;
		y1: number;
		x2: number;
		y2: number;
		edgeCount: number;
	}
	let { x1, y1, x2, y2, edgeCount }: Props = $props();
	// Thickness ∝ shared-edge count, clamped so a busy bridge stays readable.
	const width = $derived(Math.min(6, 1 + edgeCount * 0.6));
</script>

<line {x1} {y1} {x2} {y2} stroke="#e8cf8f" stroke-opacity="0.22" stroke-width={width} stroke-linecap="round" />
