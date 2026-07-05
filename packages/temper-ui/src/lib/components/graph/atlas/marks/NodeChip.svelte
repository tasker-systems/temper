<script lang="ts">
	import { docTypeHue, CANVAS_BG } from '$lib/graph/atlas/palette';
	import { truncateLabel } from '$lib/graph/atlas/labels';

	interface Props {
		x: number;
		y: number;
		r: number;
		title: string;
		docType: string | null;
		home: 'context' | 'cogmap';
		seed?: boolean;
		anchored?: boolean;
		/** Visual-only doc-type filter dimming (Task 8) — never affects the read. */
		dim?: boolean;
		onEnter?: () => void;
	}
	let { x, y, r, title, docType, home, seed = false, anchored = false, dim = false, onEnter }: Props = $props();

	const color = $derived(docTypeHue(docType));
	const filled = $derived(home === 'cogmap');
	const style = $derived(`${onEnter ? 'cursor:pointer;' : ''}opacity:${dim ? 0.15 : 1};`);
	let hovered = $state(false);
	const showLabel = $derived(anchored || hovered);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<g
	class="node-chip"
	role={onEnter ? 'button' : undefined}
	tabindex={onEnter ? 0 : undefined}
	aria-label={title}
	onclick={onEnter}
	onkeydown={(e) => e.key === 'Enter' && onEnter?.()}
	onmouseenter={() => (hovered = true)}
	onmouseleave={() => (hovered = false)}
	{style}
>
	{#if seed}
		<circle cx={x} cy={y} r={r + 6} fill="none" stroke="#cfd6e2" stroke-width="1.5" />
	{/if}
	{#if filled}
		<circle cx={x} cy={y} {r} fill={color} />
	{:else}
		<circle cx={x} cy={y} {r} fill={CANVAS_BG} stroke={color} stroke-width="2.5" />
	{/if}
	{#if showLabel}
		<text x={x} y={y + r + 13} text-anchor="middle" fill="#c7d0da" font-size="10">{truncateLabel(title, 22)}</text>
	{/if}
</g>
