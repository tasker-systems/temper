<script lang="ts">
	import { docTypeHue } from '$lib/graph/atlas/palette';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		home: 'context' | 'cogmap';
		seed?: boolean;
		onEnter?: () => void;
	}
	let { x, y, r, title, docType, home, seed = false, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(home === 'cogmap');
</script>

<g
	class="node-chip"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	style={onEnter ? 'cursor:pointer' : undefined}
>
	{#if seed}
		<circle cx={x} cy={y} r={r + 6} fill="none" stroke="#cfd6e2" stroke-width="1.5" />
	{/if}
	{#if filled}
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<circle cx={x} cy={y} {r} fill="#1b1e26" stroke={color} stroke-width="2.5" />
	{/if}
	<text x={x} y={y + r + 13} text-anchor="middle" fill="#c7d0da" font-size="10">{title}</text>
</g>
