<script lang="ts">
	import { docTypeHue, isAuthored } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		title: string;
		docType: string | null;
	}
	let { x, y, title, docType }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(isAuthored(docType));
</script>

<g class="orphan">
	{#if filled}
		<circle cx={x} cy={y} r="5" fill={color} />
	{:else}
		<circle cx={x} cy={y} r="5" fill="#1b1e26" stroke={color} stroke-width="2.5" />
	{/if}
	<text x={x + 10} y={y + 3} fill="#c7d0da" font-size="11">{title}</text>
</g>
