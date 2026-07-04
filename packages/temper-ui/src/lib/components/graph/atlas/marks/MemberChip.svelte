<script lang="ts">
	import { docTypeHue, isAuthored, CANVAS_BG } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		onEnter: () => void;
	}
	let { x, y, r, title, docType, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(isAuthored(docType));
</script>

<g
	class="member-chip"
	role="button"
	tabindex="0"
	aria-label={title}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter()}
	style="cursor:pointer"
>
	{#if filled}
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<circle cx={x} cy={y} {r} fill={CANVAS_BG} stroke={color} stroke-width="2.2" />
	{/if}
	<text x={x} y={y + r + 12} text-anchor="middle" fill="#c7d0da" font-size="10">{title}</text>
</g>
